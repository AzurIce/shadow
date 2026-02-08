一个独立的 CLI 应用，旨在解决 Git LFS 配置自定义对象存储（S3/R2/MinIO）繁琐的问题，并绕过 GitHub/GitLab 对 LFS 的带宽与空间限制。

定位：介于 Git LFS 和 DVC 之间的轻量级大文件管理工具。

## 核心决策

1.  **独立 CLI**：不作为 Git 插件（Filter）运行，操作显式化（Explicit），避免 Git 命令被阻塞。
2.  **多源设计 (Remotes)**：支持配置多个存储后端（Origin），默认使用 `origin`。
3.  **模式匹配追踪**：使用配置文件定义需要追踪的文件模式（Glob Patterns），而非仅靠手动添加。

## 架构设计

### 1. 配置管理 (Configuration)

工具在项目根目录维护 `.shadow/` 目录或单个配置文件。

**配置文件 `.shadowtrack` (类似 .gitattributes)**
定义哪些文件**应该**被 shadow 管理。这使得 CLI 可以运行 `status` 并提示：“嘿，你有一个新出的 `model.pt` 匹配了规则，但还没被 shadow 化”。

```gitignore
# .shadowtrack 示例
# 追踪所有 .psd 文件
*.psd
# 追踪 models 目录下的所有 .bin 文件
models/**/*.bin
```

**配置文件 `.shadow/config` (类似 .git/config)**
存储本地特定的配置，如 endpoint、bucket 等（不包含密钥）。

```toml
[core]
    auto_add_to_gitignore = true  # shadow add 时自动将原文件写入 .gitignore

[remote "origin"]
    provider = "s3"
    endpoint = "https://<account id>.r2.cloudflarestorage.com"
    bucket = "my-project-assets"
    region = "auto"
    # access_key 等敏感信息建议从环境变量或系统级凭证读取
```

### 2. 文件追踪与忽略机制 (核心逻辑)

基于**“Shadow 是 Git 的互补”**这一原则：Shadow 管理的文件通常**必然**在 `.gitignore` 中。

**扫描/过滤策略：互补扫描**

Shadow CLI 应当主要关注**被 Git 忽略**的文件。

1.  **基础假设**：如果一个文件没有被 `.gitignore` 忽略，它归 Git 管，Shadow 应当**跳过**（除非用户强制 `-f`）。
2.  **筛选逻辑**：
    *   扫描工作区时，Shadow 关注那些**匹配了 `.gitignore`** 的路径。
    *   在这些“Git 盲区”中，如果文件匹配 `.shadowtrack`，则**捕获**（Track）。
    *   如果不匹配 `.shadowtrack`，则**丢弃**（视为真正的垃圾文件，如 `tmp/`）。

3.  **目录性能优化 (Directory Barriers)**：
    *   为了防止扫描巨大的垃圾目录（如 `node_modules/`），采用**按需穿透**策略。
    *   如果目录被 Git 忽略（如 `build/`）：
        *   默认**不进入**。
        *   仅当 `.shadowtrack` 中有规则**显式涉及**该目录（如 `build/**/*.apk`）时，才进入扫描。
        *   *通用通配符（如 `*.apk`）不足以穿透目录忽略屏障，必须带路径。*

**总结**：`.gitignore` 是 Shadow 的**搜索范围**（Search Scope），而 `.shadowtrack` 是在这个范围内的**过滤器**（Filter）。

### 3. 文件状态定义 (Status)

运行 `shadow status` 时，文件可能处于以下状态：

*   **Untracked**: 匹配 `.shadowtrack` 规则，但没有对应的 `.shadow` 指针文件。（提示用户运行 `shadow add`）
*   **Shadowed**:
    *   `Synced`: 本地源文件存在且 Hash 与指针文件一致，且（可选）云端存在对象。
    *   **Modified**: 本地源文件 Hash 变了，指针文件是旧的。（提示用户运行 `shadow add` 更新指针）
    *   **Missing Body**: 有指针文件，但本地没有源文件。（提示用户 `shadow pull`）
    *   **Orphaned**: 有指针文件，但 `.shadowtrack` 规则不再覆盖它。

### 4. 存储模型 (Storage Architecture)

Shadow 采用**瘦指针 (Thin Pointer) + 元数据仓库 (Metadata Store)** 的分离设计。

#### A. 瘦指针 (Thin Pointer) - 提交到 Git
位于工作区，仅作为引用锚点，保持极致简洁。
*   **路径**: `src/models/bert.pt.shadow`
*   **内容**: 仅包含对象的 Hash 值（字符串）。
    ```text
    sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    ```

#### B. 元数据仓库 (Metadata Store) - 提交到 Git
位于 `.shadow/objects/`，作为所有历史对象的**全量信息库**。
*   **路径**: `.shadow/objects/e3/b0c442...` (内容寻址)
*   **内容 (JSON)**:
    ```json
    {
      "hash": "sha256:e3b0c442...",
      "size": 104857600,
      "algorithm": "sha256",
      "origin_path": "src/models/bert.pt",  // 仅记录首次创建时的路径，供参考
      "created_at": 1700000000
    }
    ```
*   **作用**: 可追溯性、去重存储元数据、作为 bucket 上传清单。

#### C. 实体文件 (Workspace Entity) - 被 Git 忽略
源文件**保留在工作区原地**，不做移动，方便用户直接编辑。
*   `shadow add` 仅计算哈希并生成上述 A 和 B。
*   `shadow pull` 仅当工作区文件缺失或哈希不匹配时，直接下载覆盖（或报错提示）。
*   *注*：暂不引入额外的本地 Blob 缓存（以节省本地空间），依赖云端作为仓库。

#### E. 本地 Blob 缓存 (Local Cache) - 可选优化
位于 `.shadow/cache/objects/`，内容寻址。
*   **作用**: 避免切换分支时重复下载。
*   **策略**:
    *   `pull`: Remote -> Cache -> Workspace (Copy/Link)。
    *   `push`: Workspace -> Remote (不强制写入 Cache，除非用户显式要求缓存)。

### 5. 详细工作流 (Revised)

1.  `shadow add file.bin`:
    *   计算 `file.bin` 哈希 -> `abc...`。
    *   **Metadata**: 检查 `.shadow/objects/ab/c...` 是否存在，不存在则写入 JSON 信息。
    *   **Pointer**: 创建 `file.bin.shadow` 写入 `sha256:abc...`。
    *   **Git**: 自动将 `file.bin` 加入 `.gitignore`。
2.  `shadow push`:
    *   遍历工作区所有 `.shadow` 文件。
    *   读取 Hash，并验证 `.shadow/objects/` 中是否有对应元数据。
    *   检查 Remote 是否存在对应 Hash 对象。
    *   若不存在，**直接上传工作区的实体文件**。
3.  `shadow pull`:
    *   读取 `.shadow` 文件获取 Hash。
    *   **Cache Check**: 检查 `.shadow/cache/objects/` 是否有该 Blob。
        *   **Hit**: 直接从 Cache 复制/硬链接到工作区。
        *   **Miss**: 从 Remote 下载到 Cache，再复制/硬链接到工作区。
    *   若工作区文件已存在且 Hash 匹配，则跳过（Up-to-date）。

## 待定问题

*   **鉴权管理**：是在这个 CLI 里做 `login` 命令，还是完全依赖环境变量（`AWS_ACCESS_KEY_ID`）？建议初期依赖环境变量或 `~/.aws/credentials`，减少开发成本。

