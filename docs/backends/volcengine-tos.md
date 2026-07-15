# 火山引擎 TOS 后端设计

## 1. 目标

火山引擎对象存储 TOS 是 Shadow v2 的首个存储后端。第一阶段只要求实现通用 `BlobStore` 所需的对象查询、上传和下载，不实现远端 GC。

实现优先使用火山引擎官方 Rust SDK：

```toml
ve-tos-rust-sdk = { version = "2.9", default-features = false, features = ["asynchronous", "tokio-runtime", "use-rustls"] }
```

选择官方 SDK 的原因：

- 原生支持 TOS 认证和错误模型。
- 提供异步对象操作。
- 支持从文件上传、下载到文件、HEAD、ListObjectsV2 和批量删除。
- 后续可以使用 multipart API，而不依赖 S3 兼容细节。

具体依赖版本在实现时锁定并经过最小兼容性验证，不在核心模型中暴露 SDK 类型。

## 2. 命名空间选型

火山引擎 TOS 提供两种桶类型：

| 类型 | 官方定义与特点 | 对 Shadow 的价值 |
| --- | --- | --- |
| FNS 平铺命名空间 | 对象存在于扁平 key 空间，目录通常由 key 前缀和 `/` 分隔符模拟 | 与 S3、R2、MinIO 等通用对象存储模型一致 |
| HNS 分层命名空间 | 基于分层元数据管理，目录具有真实语义，优化目录 `HEAD/List`，支持高性能、原子性的目录和文件 rename | 主要面向 HDFS、大数据、数据湖和需要目录操作的场景 |

根据火山引擎官方《对象存储文档指南》5.17，HNS 的核心优势是目录级 `mv/rename`、目录查询以及对象语义与文件语义互通。它还存在地域、功能支持和开通方式上的额外限制。

Shadow 选择 **FNS 平铺桶**：

1. Shadow 对象按 SHA-256 寻址，不需要移动或重命名远端对象。
2. 对象一旦上传即不可变，不需要目录级事务。
3. 远端目录只用于方便观察 key，`/` 在逻辑上只是普通分隔字符。
4. S3 和多数对象存储原生采用平铺 key 空间，FNS 更利于后续增加通用后端。
5. HNS 带来的目录性能和 HDFS 兼容能力不会改善 Shadow 的核心工作流。

TOS backend 明确不能依赖以下 HNS 特有能力：

- 真实目录对象
- 目录 `HEAD` 或 `GetFileStatus`
- `RenameObject`
- `ModifyObject`
- 回收站
- `x-tos-directory` 等目录元数据
- HDFS 兼容访问

允许使用的能力应当都能映射到普通对象存储：

- HEAD Object
- Put Object
- Get Object
- Multipart Upload
- List Objects with prefix
- Delete Object / batch delete

第一阶段只正式支持 FNS 桶。若 backend check 能获取桶类型，应在发现 HNS 时返回清晰的 Unsupported 错误，而不是启用 HNS 特殊逻辑。未来即使验证 HNS 可以兼容，也只能通过通用 `BlobStore` 操作使用它。

调查来源：[火山引擎对象存储文档指南](https://eps-common-private-online.tos-cn-beijing.volces.com/cloud-doc/eps-doc-center-pdf/%E5%AF%B9%E8%B1%A1%E5%AD%98%E5%82%A8_%E6%96%87%E6%A1%A3%E6%8C%87%E5%8D%97_1784086371.pdf) 5.17“分层命名空间（HNS）”及官方 Rust SDK 中对应的 HNS API。

## 3. 配置

项目共享配置保存在仓库根目录 `shadow.toml`：

```toml
version = 1
name = "my-models"

[backend]
type = "volcengine_tos"
endpoint = "https://tos-cn-beijing.volces.com"
region = "cn-beijing"
bucket = "example-shadow"
prefix = "shadow"
```

配置要求：

- `name`：项目在 bucket/prefix 下的唯一名称，首次发布后不应直接修改。
- `endpoint`：显式配置，避免在核心代码中维护区域映射表。
- `region`：用于 SDK 和签名。
- `bucket`：必须已存在，v1 不负责创建 bucket。
- `prefix`：允许一个 bucket 服务多个应用，规范化后不能以 `/` 开头。

密钥不得写入提交配置。第一阶段从环境变量读取：

```text
TOS_ACCESS_KEY
TOS_SECRET_KEY
TOS_SECURITY_TOKEN     # 可选
```

这些名称与官方 SDK 的环境凭证 provider 保持一致。未来再支持实例角色、STS 和自定义凭证进程。

日志和错误信息不得打印 Access Key、Secret Key、Session Token 或完整签名请求头。

## 4. 对象键布局

完整对象键：

```text
<prefix>/<name>/objects/sha256/<first-2>/<remaining-62>
```

示例：

```text
shadow/my-models/objects/sha256/ab/cdef...
```

约束：

1. 对象键只由已验证的 `ObjectId` 生成，不能接受任意字符串。
2. 所有分隔符固定使用 `/`。
3. 对象键不包含原文件路径。
4. 同一个项目 name 命名空间中，相同 SHA-256 只保存一次。
5. v1 不启用跨仓库去重，以换取可解释且安全的 GC 边界。

## 5. 远端对象元数据

上传时可以写入以下自定义元数据：

```text
shadow-version = 1
shadow-oid = sha256:...
shadow-size = 104857600
shadow-name = my-models
```

远端元数据用于诊断，不作为唯一真相来源。对象身份仍由 key 和下载后的 SHA-256 校验确定。

不能依赖 ETag 等于内容 MD5。multipart upload 的 ETag 通常不代表整个文件的单一内容摘要。

## 6. 后端结构

建议模块布局：

```text
src/backend/
├── mod.rs
├── error.rs
├── blob_store.rs
└── volcengine_tos.rs
```

`volcengine_tos.rs` 负责：

- 将 Shadow 配置转换为 SDK client。
- 将 `ObjectId` 转换为 TOS object key。
- 调用 SDK 的 HEAD、上传、下载和未来的 list/delete API。
- 将 SDK 错误映射为 Shadow 后端错误。
- 实现重试、超时和 multipart 策略。
- 实现通用 `BlobStore`，并在未来按需实现 `BlobInventory`。

其他模块不得直接依赖 `ve_tos_rust_sdk`。

## 7. Client 生命周期

每次 CLI 运行只创建一个 TOS client，并在所有文件操作中复用。client 应封装在 `Arc` 内，允许未来受控并发上传和下载。

初始化时只验证配置格式，不立即发起网络请求。需要显式连接检查时使用：

```text
shadow backend check
```

检查内容包括：

1. 凭证是否存在。
2. bucket 是否可访问。
3. repository prefix 是否可读写。
4. endpoint、region 和 bucket 组合是否正确。

读写检查应使用独立的临时 key，并确保测试对象最终被删除。第一阶段如果暂不实现删除，可以只执行只读 bucket 检查和用户确认后的测试上传。

## 8. Stat

`stat(oid)` 使用 HEAD Object。

返回：

```rust
pub struct ObjectInfo {
    pub size: u64,
    pub etag: Option<String>,
    pub last_modified: Option<SystemTime>,
}
```

处理规则：

- 对象不存在返回 `Ok(None)`。
- 认证失败返回 `Unauthorized`，不能伪装成不存在。
- 权限不足返回 `Forbidden`。
- HEAD 返回的 size 与 ref size 不一致时报告 `IntegrityMismatch`。
- 必须基于 SDK 的结构化错误码判断，不能匹配错误字符串中的 `404`。

## 9. 上传

### 9.1 上传前检查

1. 验证 cache 文件存在。
2. 验证 cache 文件大小等于 ref size。
3. 必要时重新计算 SHA-256，确保 cache 未损坏。
4. HEAD 远端对象。
5. 已存在且 size 正确时直接成功。
6. 已存在但 size 不正确时停止，不允许覆盖异常对象。

### 9.2 小对象上传

低于 multipart 阈值的对象使用 SDK 的文件流式上传接口。禁止将完整文件读取到 `Vec<u8>`。

初始建议阈值：

```text
multipart_threshold = 64 MiB
```

阈值应允许后续通过配置调整，但不要在第一版暴露过多性能参数。

### 9.3 Multipart 上传

大对象流程：

1. CreateMultipartUpload。
2. 按固定 part size 读取本地 cache。
3. 并发 UploadPart。
4. 收集 part number 和 ETag。
5. CompleteMultipartUpload。
6. HEAD 验证最终对象大小。

初始建议：

```text
part_size = 16 MiB
max_concurrency = 4
```

实现必须限制内存上界，近似为：

```text
part_size * max_concurrency + 固定开销
```

任何 part 失败时：

- 停止调度新的 part。
- 尝试 AbortMultipartUpload。
- 保留本地 cache，不写入或更新 ref。
- 返回包含 request id 的错误。

第一阶段可以先实现可靠的单请求文件流上传，再实现 multipart；但接口和测试必须从一开始避免整体内存缓冲。

## 10. 下载

下载始终写入 `.shadow/tmp/`，不能直接覆盖 cache 或工作文件：

```text
TOS
  -> .shadow/tmp/<random>.download
  -> size 校验
  -> SHA-256 校验
  -> atomic rename 到 cache
  -> copy/reflink 到工作区临时文件
  -> atomic rename 到工作文件
```

处理要求：

- 流式下载并同步计算 SHA-256。
- 校验实际字节数等于 ref size。
- 校验摘要等于 ref oid。
- 校验失败时删除临时文件。
- 工作文件已修改时默认拒绝替换。
- 下载中断不能留下一个看似有效的 cache 对象。

第一阶段不要求断点续传。后续可以通过 Range GET 和临时状态文件实现，但必须重新验证最终完整对象。

## 11. 重试与并发

只重试可恢复错误：

- 网络连接中断
- timeout
- 429/rate limit
- 5xx 服务错误

不自动重试：

- 认证失败
- 权限不足
- bucket 不存在
- 配置错误
- 完整性校验失败

建议使用带抖动的指数退避：

```text
最大尝试次数：4
初始延迟：250 ms
最大延迟：8 s
```

是否由官方 SDK 已完成重试需要在实现时确认。不能在 SDK 重试之外无条件再套一层重试，避免请求次数成倍增加。

多文件传输初始并发建议为 4。单文件 multipart 与多文件并发必须共享总并发限制，防止同时打开过多文件和网络连接。

## 12. 错误信息

面向用户的错误至少包含：

- 操作类型，例如 HEAD、upload、download。
- bucket 和经过安全处理的 object key。
- HTTP 状态或 TOS 错误码。
- TOS request id，便于服务端排查。
- 是否可以重试。

示例：

```text
upload failed: bucket=example-shadow object=.../sha256/ab/cdef
tos_code=AccessDenied request_id=... retryable=false
```

不能把对象不存在、权限不足和网络故障统一显示为“文件不存在”。

## 13. GC 能力预留

未来 TOS GC 需要：

- ListObjectsV2，限定 repository prefix。
- 分页处理 continuation token。
- 读取 object size 和 last modified。
- DeleteMultiObjects 分批删除。
- 对每批结果分别记录成功和失败对象。

GC 不依赖上传清单文件。TOS prefix 下的对象列表就是库存集合。

第一阶段即使不暴露 GC 命令，也应确保 object key 和项目 name 命名空间已经满足未来安全列举与删除的要求。

## 14. 测试策略

### 14.1 单元测试

- 配置解析和缺失字段。
- ObjectId 到 object key 的映射。
- SDK 错误到 Shadow 错误的映射。
- multipart part size 和边界计算。
- repository prefix 规范化。

### 14.2 本地集成测试

通过 fake `BlobStore` 验证核心状态机：

- 对象已存在时跳过上传。
- 上传失败不创建或更新 ref。
- 上传成功但 ref 写入失败时可重试。
- 下载摘要错误时不写入 cache。
- 本地修改文件不会被普通 restore 覆盖。

### 14.3 TOS 集成测试

使用专用测试 bucket 和随机 repository prefix：

- HEAD 不存在对象。
- 小文件上传与下载。
- 空文件。
- 大文件或 multipart。
- 重复上传同一 oid。
- 无效凭证。
- 只读凭证。
- 下载后 SHA-256 一致。

真实 TOS 测试默认标记为 ignored，只有环境变量完整时才运行，避免普通 `cargo test` 产生费用或访问外部服务。

## 15. 实现顺序

建议按以下顺序实现：

1. 定义 `ObjectId`、`BlobKey`、`BlobMetadata`、`BlobStore` 和统一错误。
2. 完成 TOS 配置解析和 client 创建。
3. 实现 object key 映射。
4. 实现 HEAD/stat。
5. 实现文件流上传。
6. 实现下载到临时文件。
7. 接入 `shadow publish/restore` 状态机。
8. 增加真实 TOS opt-in 集成测试。
9. 实现 multipart upload。
10. 最后增加 list/delete，为 GC 做准备。
