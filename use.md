# RDL (Rust Downloader) 用户指南

RDL (Rust Downloader) 是一个高性能、支持断点续传、多线程并发的通用下载工具。它特别针对大文件下载进行了优化，并内置了对 ModelScope 等模型仓库的便捷支持。

## ✨ 主要特性

*   **多线程并发**：支持多文件并发下载及单文件多线程分片下载。
*   **断点续传**：自动记录下载进度，中断后重启即可无缝续传。
*   **哈希校验**：支持 SHA256 校验，确保文件完整性。
*   **模型仓库支持**：内置 ModelScope 支持，可直接拉取模型文件列表并下载。
*   **后台守护**：支持 Unix 系统下的后台守护进程模式。

## 🚀 快速开始

### 1. 基础下载

在当前目录下创建一个 `download.txt` 文件，每行一个 URL：

```text
https://example.com/file1.zip
https://example.com/file2.bin|sha256_hash_here
```

运行工具：

```bash
rdl
```

工具将自动读取 `download.txt` 并下载文件到 `downloads` 目录。

### 2. 下载 ModelScope 模型

直接指定模型名称，工具会自动获取文件列表并开始下载：

```bash
rdl --fetch-list Qwen/Qwen3-Next-80B-A3B-Instruct
```

这将自动生成清单文件并下载到 `downloads/modelscope/Qwen/Qwen3-Next-80B-A3B-Instruct/` 目录。

---

## 📖 详细使用指南

### 常用参数

| 参数 | 简写 | 说明 | 默认值 |
| :--- | :--- | :--- | :--- |
| `--tasks-file` | `-t` | 任务清单文件路径 | `download.txt` |
| `--download-dir` | `-d` | 下载保存目录 | `downloads` |
| `--concurrency` | `-c` | 同时下载的文件数量 | CPU 核心数 |
| `--split` | `-s` | 单个文件的分片线程数 | 8 |
| `--rate-limit` | `-r` | 全局限速 (字节/秒) | 无限制 |
| `--verify-hash` | | 校验模式 (`auto`, `on`, `off`) | `auto` |

### 进阶场景

#### 自定义输入输出

```bash
rdl -t my_list.txt -d /data/models
```

#### 性能调优

如果你的网络带宽很大，可以适当增加并发数和分片数：

```bash
# 4个文件同时下载，每个文件拆分16个线程
rdl -c 4 -s 16
```

#### 限速下载

限制最大下载速度为 10MB/s (10 * 1024 * 1024 = 10485760)：

```bash
rdl -r 10485760
```

#### 校验策略 (`--verify-hash`)

*   `auto` (默认): 如果清单中提供了哈希值则校验，否则跳过。
*   `on`: 强制校验。如果清单中缺少哈希值会报错。
*   `off`: 不进行校验。

### 后台运行 (Unix Only)

在 Linux/macOS 上，你可以让工具在后台运行：

*   **启动守护进程**: `rdl --daemon`
*   **查看实时日志**: `rdl --follow`
*   **查看任务状态**: `rdl --list`
*   **停止任务**: `rdl --stop`
*   **暂停/恢复**: `rdl --pause` / `rdl --resume`

## 💡 常见问题

**Q: 下载中断了怎么办？**
A: 直接重新运行相同的命令即可。工具会检测 `.part` 和 `.part.json` 文件，自动从上次中断的地方继续下载。

**Q: 如何生成带哈希的任务列表？**
A: 任务文件格式为 `URL|HASH`。如果是 ModelScope，使用 `--fetch-list` 会自动生成带哈希的列表。

**Q: 部署建议？**
A: 建议将编译好的二进制文件放入系统 PATH (如 `/usr/local/bin`)。在生产环境中使用时，建议显式指定绝对路径的 `--tasks-file` 和 `--download-dir`。
