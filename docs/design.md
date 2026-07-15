# Shadow v2 总体设计

## 1. 定位

Shadow 是一个显式操作的大文件版本管理工具。Git 负责提交小型引用文件，Shadow 负责将真实文件保存到对象存储，并在工作区中恢复这些文件。

Shadow 不使用 Git clean/smudge filter，不阻塞普通 Git 命令。用户直接编辑 `.gitignore` 声明管理范围，再通过 `shadow status`、`shadow publish` 和 `shadow restore` 检查、发布和恢复大文件。

首个版本只支持一个火山引擎 TOS 后端，但核心层不依赖具体存储实现。

## 2. 核心结论

被 Shadow 管理的工作文件必须被 Git 忽略：

```text
ShadowManaged(path) -> GitIgnored(path)
```

反向关系不成立。被 Git 忽略的构建产物、密钥和临时文件不一定由 Shadow 管理。

Shadow 通过根目录 `.gitignore` 的尾部区域声明管理规则：

```gitignore
/target/
.env

# shadow
/assets/**/*.png
/models/**/*.bin
```

严格约定：

1. 标记必须是独立一行，内容为 `# shadow`。
2. 标记之后直到文件结尾都属于 Shadow 区域。
3. Shadow 区域必须位于根目录 `.gitignore` 的最后部分。
4. 标记后的内容使用标准 Git ignore 语义，包括空行、注释、转义和 `!` 否定规则。
5. 对一个路径而言，Shadow 区域中最后一条匹配规则决定它是否属于 Shadow 管理范围。
6. 标记只能出现一次，重复出现属于配置错误。
7. 标记不存在表示当前没有 Shadow 管理规则。
8. Shadow 最终仍需验证匹配文件确实被 Git 有效忽略。

Shadow 不提供 `add` 或 `track` 命令。用户直接编辑 `.gitignore`，Shadow 只解释标记之后的规则。Shadow 区域表示管理策略，`.shadow/refs/` 表示具体文件已经发布的事实。

## 3. 核心不变量

实现必须始终维护以下不变量：

1. 每个 ref 都包含合法且规范化的内容对象 ID。
2. 已发布 ref 对应的远端对象必须存在。
3. 每个 Shadow 工作文件必须被最终生效的 Git ignore 规则覆盖。
4. cache 中的对象内容必须与其对象 ID 一致。
5. 对象一经写入就不可修改，只能按对象 ID 新建另一个对象。
6. 网络或进程失败不能产生指向未完成对象的 ref。
7. 下载失败或校验失败不能破坏现有工作文件。

## 4. 仓库布局

```text
shadow.toml
.shadow/
├── .gitignore
├── refs/
│   └── assets/
│       └── image.png.ref
├── cache/
│   └── objects/
│       └── sha256/
│           └── ab/
│               └── cdef...
└── tmp/
```

需要提交到 Git：

- `shadow.toml`
- `.shadow/.gitignore`
- `.shadow/refs/**/*.ref`

只在本地存在：

- `.shadow/cache/`
- `.shadow/tmp/`

`.shadow/.gitignore` 默认内容：

```gitignore
/cache/
/tmp/
```

`shadow.toml` 是项目级配置和 Shadow 仓库的主标识，放在 Git 仓库根目录，便于用户发现和编辑。`.shadow/` 只保存引用和本地运行数据，不再保存项目配置。

仓库发现流程：

1. 通过 Git 确定仓库根目录。
2. 检查根目录是否存在 `shadow.toml`。
3. 检查 `.shadow/refs/` 等内部结构，缺失时给出修复建议。

`shadow.toml` 只能保存可提交、可共享的配置。访问密钥来自环境变量或用户级凭证配置，不能写入项目文件。未来如果需要个人覆盖配置，应放在操作系统用户配置目录，而不是 `.shadow/`。

## 5. 工作文件与 ref 映射

工作文件路径按相同目录结构映射到 `.shadow/refs/`：

```text
assets/image.png
    -> .shadow/refs/assets/image.png.ref

models/bert/model.bin
    -> .shadow/refs/models/bert/model.bin.ref
```

路径必须先进行规范化：

1. 转换为相对仓库根目录的路径。
2. 拒绝绝对路径和包含有效 `..` 逃逸的路径。
3. 拒绝指向仓库外部的符号链接目标，除非未来明确支持。
4. ref 中使用 `/` 作为稳定路径分隔符。
5. Windows 下比较路径时要正确处理盘符和大小写规则。

## 6. Ref 格式

ref 使用稳定、可读、可扩展的 TOML 格式：

```toml
version = 1
oid = "sha256:abcdef0123456789..."
size = 104857600
```

字段语义：

- `version`：ref 格式版本。
- `oid`：算法名称和十六进制摘要。
- `size`：原始文件字节数。

工作区路径由 ref 自身的位置表达，不在内容中重复保存。remote 也不写入 ref，同一 ref 将来可以上传到多个后端。

ref 写入必须采用临时文件加原子重命名，序列化结果必须稳定。

## 7. 对象 ID 与存储键

v1 只支持 SHA-256：

```text
sha256:<64 lowercase hex characters>
```

本地 cache 路径：

```text
.shadow/cache/objects/sha256/ab/cdef...
```

远端对象键：

```text
<name>/objects/sha256/ab/cdef...
```

`name` 保存在根目录 `shadow.toml`，`shadow init` 默认使用 Git 仓库目录名。它用于隔离共享 bucket/prefix 中的不同项目，同一 bucket/prefix 下必须保持唯一。对象键不包含原始文件路径，因此文件重命名不会重新上传内容。

项目首次发布后不应直接修改 `name`。修改 name 相当于切换到一个新的远端命名空间，已有 refs 将无法在新位置找到旧对象，除非重新发布或执行专门的迁移。

## 8. 本地缓存

cache 是内容寻址的本地对象库，承担以下职责：

- 保存 `shadow publish` 上传前的稳定内容快照。
- 支持发布失败后重试和相同内容去重。
- 避免切换分支时重复下载。
- 为 CI 提供稳定缓存目录。
- 在写入工作区之前完成远端下载和哈希校验。

cache 对象必须视为不可变。恢复工作文件优先使用 reflink，其次使用普通复制。不能直接创建可写硬链接，否则修改工作文件会同时破坏 cache 对象。

cache 写入流程：

1. 在 `.shadow/tmp/` 创建临时文件。
2. 流式读取源文件，同时计算 SHA-256 并写入临时文件。
3. 校验读取长度。
4. 将临时文件原子移动到对应 cache 路径。
5. 如果对象已经存在，则验证已有对象并丢弃临时文件。

## 9. 状态模型

一个路径可能处于以下状态：

| 状态 | 含义 |
| --- | --- |
| Unpublished | 工作文件存在并匹配 Shadow 区域，但没有 ref |
| Published | 工作文件与 ref 指向相同内容 |
| Modified | 工作文件与 ref 都存在，但内容哈希不同 |
| Missing | ref 存在，但工作文件不存在 |
| Orphaned | ref 存在，但路径不再匹配 Shadow ignore 区域 |
| CacheMissing | ref 存在，但本地 cache 没有对象 |
| RemoteMissing | ref 存在，但远端没有对象，仅在联网检查时报告 |

普通 `shadow status` 不访问网络。`shadow status --remote` 才查询对象存储。

`Modified` 是有意保持方向不确定的状态。Shadow 无法仅凭两个不同的哈希判断应该上传工作文件，还是应该用 ref 覆盖工作文件。用户必须显式运行 `publish` 或 `restore --force`。

## 10. 命令语义

### `shadow init`

1. 确认当前目录属于 Git 仓库。
2. 创建 `.shadow/` 目录结构。
3. 使用 Git 仓库目录名生成默认 `name`。
4. 创建根目录 `shadow.toml` 和 `.shadow/.gitignore`。
5. 在根 `.gitignore` 末尾创建 `# shadow` 标记。

### `shadow status [paths...]`

`status` 是整个工作流的入口。它将 Shadow 区域匹配到的工作文件与 `.shadow/refs/` 中的引用进行配对，并报告：

- `Unpublished`：有工作文件，没有 ref，建议 `shadow publish`。
- `Published`：工作文件哈希与 ref 一致。
- `Modified`：两者都存在但哈希不同，由用户选择 publish 或 restore。
- `Missing`：有 ref，没有工作文件，建议 `shadow restore`。
- `Orphaned`：有 ref，但路径不再属于 Shadow 区域。

默认只检查本地工作文件、ref 和 cache。`--remote` 额外通过后端查询 ref 对应对象是否存在和大小是否正确。

### `shadow publish [paths...]`

`publish` 以当前工作文件为真，负责把本地内容变成可由 ref 引用的已发布对象：

1. 扫描 Shadow 区域匹配的工作文件，可由路径参数限制范围。
2. 处理 `Unpublished` 和 `Modified` 文件；`Published` 文件直接跳过。
3. 将工作文件流式导入 cache，同时计算 SHA-256 和大小。
4. 使用 hash 生成远端对象 key。
5. 调用后端 `stat` 检查对象是否存在。
6. 对象已存在且大小正确时跳过上传。
7. 对象不存在时从 cache 流式上传，大对象由后端实现 multipart。
8. 上传完成后再次检查远端对象。
9. 只有远端对象确认可用后，才原子写入或更新 `.shadow/refs/<path>.ref`。

因此，同一内容即使出现在多个路径中，也只上传一次。上传成功但本地 ref 写入失败时，再次运行 publish 会通过远端 `stat` 跳过上传并补写 ref。

### `shadow restore [paths...]`

`restore` 以 ref 为真，默认只恢复 `Missing` 文件：

1. 遍历选中的 refs。
2. cache 命中时验证大小和 SHA-256。
3. cache 未命中时调用后端下载到 `.shadow/tmp/`。
4. 校验下载大小和 SHA-256 后原子写入 cache。
5. 从 cache 复制或 reflink 到工作区临时文件。
6. 原子移动到目标工作路径。

工作文件已经存在但与 ref 不同时，普通 restore 必须拒绝覆盖。只有显式传入 `--force` 才允许以 ref 替换工作文件。

### `shadow remove <paths...>`

删除 published ref，但默认保留工作文件和远端对象。管理规则由用户手动维护，命令不修改 `.gitignore`。

### `shadow verify`

验证 refs、cache、工作文件和 Git ignore 不变量。`--remote` 额外验证远端对象。

## 11. 后端抽象

核心执行逻辑不能出现 TOS、S3、bucket、ETag 或厂商错误类型。它只依赖一个面向不可变 blob 的窄接口：

```rust
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn stat(&self, key: &BlobKey) -> BackendResult<Option<BlobMetadata>>;

    async fn upload_file(
        &self,
        key: &BlobKey,
        source: &Path,
        size: u64,
    ) -> BackendResult<()>;

    async fn download_file(
        &self,
        key: &BlobKey,
        destination: &Path,
    ) -> BackendResult<()>;
}
```

`BlobKey` 是由核心层生成并验证的相对对象键。backend 只负责将它放到配置的 bucket/prefix 下。核心层不要求后端具有真实目录、原子 rename、对象标签或厂商特有校验和。

使用文件路径作为传输边界是有意的：Shadow 的 cache 本身就是文件存储，后端可以直接进行流式文件传输和 multipart，不需要把整个对象读入内存。以后如果出现非文件输入需求，再增加流式接口，不提前引入复杂的异步 trait 生命周期。

GC 使用独立的可选接口：

```rust
#[async_trait]
pub trait BlobInventory: Send + Sync {
    async fn list_prefix(&self, prefix: &BlobKeyPrefix) -> BackendResult<BlobStream>;
    async fn delete_batch(&self, keys: &[BlobKey]) -> BackendResult<DeleteResult>;
}
```

日常 `status/publish/restore` 只需要 `BlobStore`。某个后端没有实现 `BlobInventory` 时，核心功能仍然可用，只是不支持远端 GC。

命令执行层只接受 trait object：

```rust
pub struct PublishService {
    store: Arc<dyn BlobStore>,
}

pub struct RestoreService {
    store: Arc<dyn BlobStore>,
}
```

后端工厂负责读取 `[backend]` 配置并构造具体实现。除工厂和 `src/backend/volcengine_tos.rs` 外，代码中不应出现 `TosClient` 或任何 TOS SDK 类型。普通本地 `status` 不创建 backend；只有 `status --remote`、`publish` 和 cache miss 时的 `restore` 需要 `BlobStore`。

错误需要归一化为稳定类别：

- `NotFound`
- `Unauthorized`
- `Forbidden`
- `RateLimited`
- `Timeout`
- `IntegrityMismatch`
- `BackendUnavailable`
- `Other`

## 12. 远端保留与 GC

v1 采用 append-only：上传对象后，普通命令不删除远端对象。删除 ref 不等于删除对象。

未来 GC 使用集合差：

```text
远端仓库命名空间中的全部对象
    -
Git 所有可达 branch、tag 和 commit 中 ref 引用的对象
    =
GC 候选对象
```

不维护提交到 Git 的“所有曾上传对象列表”。对象存储自身的 list API 就是远端库存来源。

GC 至少需要：

- 默认 `--dry-run`
- 宽限期，例如 30 天
- 项目 name 命名空间隔离
- 删除前再次计算可达集合
- 分批删除与失败清单
- 不自动重写 Git 历史

永久删除 Git 历史中的 ref 属于独立的高风险 `purge` 能力，不属于普通 GC。

## 13. 第一阶段范围

第一阶段实现以下闭环：

1. `init`
2. Git ignore Shadow 区域解析
3. ref 和 cache
4. 本地 `status/verify`
5. 火山引擎 TOS 的 `BlobStore`
6. `publish`
7. `restore`

以下能力延后：

- 多后端配置和多 remote
- 远端 GC
- 历史重写与 purge
- 自动 Git hooks
- 跨仓库对象共享
- 后台并行传输服务
