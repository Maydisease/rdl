# RDL (Rust Downloader)

RDL 是一个高性能、轻量级的命令行下载工具，专为大文件下载和模型仓库（如 ModelScope）设计。它基于 Rust 编写，提供极快的下载速度和稳定的断点续传能力。

## ✨ 核心特性

*   **🚀 高性能并发**：支持多文件并发下载，单文件多线程分片下载，充分利用带宽。
*   **🔄 断点续传**：自动记录下载进度，随时中断，随时继续，无需重新下载。
*   **🛡️ 安全可靠**：支持 SHA256 哈希校验，确保文件完整性。
*   **📦 模型仓库支持**：内置 ModelScope 支持，一键拉取并下载整个模型仓库。
*   **👻 后台守护**：支持 Unix 系统下的后台守护进程模式，方便长期任务管理。

## 🛠️ 安装

目前建议通过源码编译安装： 

```bash
# 1. 克隆仓库
git clone https://github.com/your-repo/rdl.git
cd rdl

# 2. 编译 Release 版本
cargo build --release

# 3. (可选) 将二进制文件移动到 PATH
cp target/release/rdl /usr/local/bin/
```

## 📖 快速上手

### 1. 单文件下载

```bash
rdl https://example.com/file.zip
```

### 2. 批量下载

创建一个包含 URL 的 `download.txt` 文件，然后运行：

```bash
rdl
```

### 3. 下载 ModelScope 模型

```bash
rdl --fetch-list Qwen/Qwen3-Next-80B-A3B-Instruct
```

## 📚 文档导航

*   **[用户指南 (User Guide)](use.md)**: 详细的参数说明、配置选项和使用场景。
*   **[开发文档 (Developer Guide)](use.dev.md)**: 架构设计、代码结构和贡献指南。

## 📄 License

MIT