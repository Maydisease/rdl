# RDL (Rust Downloader) 开发文档

本文档面向项目的开发人员和贡献者，详细介绍了 RDL (Rust Downloader) 的架构设计、代码结构以及开发流程。

## 🛠️ 开发环境搭建

### 前置要求
*   Rust (最新稳定版)
*   Cargo

### 构建与运行

```bash
# 开发模式运行
cargo run -- --help

# 编译 Release 版本
cargo build --release

# 运行测试
cargo test
```

## 🏗️ 架构概览

本项目采用模块化设计，核心逻辑与 CLI 交互分离。主要由以下几个模块组成：

### 1. 核心模块 (`src/`)

*   **`main.rs`**: 程序入口。负责参数解析 (使用 `clap`)，根据参数分发到同步命令 (如 `list`, `stop`) 或异步下载任务。
*   **`cli.rs`**: 定义 CLI 参数的数据结构和枚举 (如 `VerifyMode`)。
*   **`commands.rs`**: 业务逻辑层。协调下载流程，包括读取任务文件、预计算总大小、初始化 `Downloader` 以及处理守护进程指令。
*   **`downloader.rs`**: 核心下载引擎。
    *   管理全局并发 (`Semaphore`) 和速率限制 (`governor`)。
    *   实现单文件下载逻辑：检查本地状态 -> 分片 -> 并发下载 -> 合并/重命名。
    *   处理断点续传逻辑。
*   **`state.rs`**: 定义下载状态的数据结构 (`DownloadState`, `PartState`)，负责序列化/反序列化 `.part.json` 文件。
*   **`hashing.rs`**: 提供 SHA256 哈希计算功能，用于文件完整性校验。
*   **`daemon.rs`**: (Unix Only) 封装守护进程逻辑，包括 fork、PID 文件管理、信号处理。
*   **`providers/`**: 模型仓库适配层。
    *   `mod.rs`: 统一接口定义。
    *   `modelscope.rs`: ModelScope API 的具体实现。

### 2. 关键流程解析

#### A. 下载流程 (`commands::run_downloads`)
1.  **解析任务**: 读取 `--tasks-file`，解析 URL 和可选的 Hash。
2.  **预检 (Pre-flight)**: 并发发送 HEAD 请求获取文件大小 (`get_total_size`)，用于显示总进度。
3.  **初始化**: 创建 `Downloader` 实例，配置并发数、限速器。
4.  **并发调度**: 使用 `tokio::spawn` 和 `Semaphore` 控制文件级别的并发下载。
5.  **单文件处理 (`Downloader::download_file`)**:
    *   **检查**: 检查目标文件是否存在。
    *   **状态恢复**: 读取 `.part.json` 恢复分片状态，或初始化新状态。
    *   **分片下载**: 根据 `--split` 将文件切分为多个 Range 请求。
    *   **写入**: 多线程写入同一个文件的不同位置 (使用 `SeekFrom::Start`)。
    *   **持久化**: 定期更新 `.part.json` 以支持断点续传。
    *   **完成**: 下载完成后校验 Hash (如果需要)，删除临时文件，重命名为最终文件名。

#### B. 守护进程 (`daemon.rs`)
*   使用 `daemonize` crate 将进程转入后台。
*   通过 PID 文件 (`/tmp/rdl.pid`) 管理进程生命周期。
*   支持 `SIGTERM` (停止), `SIGTSTP` (暂停), `SIGCONT` (恢复) 信号。

## 📂 目录结构说明

```text
src/
├── main.rs          # 入口 & 参数解析
├── cli.rs           # CLI 类型定义
├── commands.rs      # 高层命令实现 (run, list, fetch)
├── downloader.rs    # 核心下载器实现
├── state.rs         # 状态持久化结构
├── hashing.rs       # 哈希计算
├── utils.rs         # 通用工具函数
├── daemon.rs        # 守护进程管理
└── providers/       # 第三方源适配
    ├── mod.rs       # Provider trait 定义
    └── modelscope.rs
```

## 🔌 扩展指南

### 添加新的 Provider

如果需要支持新的模型仓库 (如 HuggingFace)，请遵循以下步骤：

1.  在 `src/providers/` 下创建新文件 (e.g., `huggingface.rs`)。
2.  实现获取文件列表的逻辑，返回 `Vec<DownloadItem>`。
3.  在 `src/providers/mod.rs` 中注册新的模块和匹配逻辑。
4.  在 `src/commands.rs` 的 `generate_download_list` 中适配新的 Provider 参数。

## 📝 调试技巧

*   **日志**: 目前主要通过 `println!`/`eprintln!` 输出。在守护进程模式下，标准输出会被重定向到日志文件，可以使用 `--follow` 查看。
*   **状态文件**: 下载过程中的 `.part.json` 是明文 JSON，可以直接查看以调试分片状态。
*   **单线程调试**: 将并发数设为 1 (`-c 1 -s 1`) 可以简化调试流程，避免多线程竞态干扰。
