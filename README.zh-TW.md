# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-blue)]
[![Version](https://img.shields.io/badge/version-0.2.0-informational)](Cargo.toml)

[English](README.md) | [日本語](README.ja.md) | 繁體中文

**stoker 是單機、Git-aware 的任務排程器。**
只要任務可以由 Git commit 固定程式碼版本並以 command 執行，就能排程；AI training 只是最初的範例。每個 Job 會在獨立 Git worktree 中執行，避免影響目前的工作目錄。

## 快速開始

需要 Rust、Git，以及一個 Git repository：

```bash
# 從 crates.io 安裝 CLI（需要 Rust 與 Git）
cargo install stoker-engine

# 啟動背景 scheduler（Linux 與 Windows 皆適用）
stoker serve

# 在 Git repository 內建立 DRAFT Job
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# 確認內容後加入 queue（<JOB_ID> 由上一個指令輸出）
stoker show <JOB_ID>
stoker commit <JOB_ID>

# 查詢與管理
stoker status
stoker jobs
stoker jobs --user alice
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>
stoker cancel <JOB_ID>
stoker stop
stoker --version
stoker update
stoker uninstall
```

`--cmd` 後面的內容會完整傳給要執行的程序；請把 stoker 自己的選項放在 `--cmd` 前。

`--user` 是 stoker 的邏輯 owner 標籤，不是作業系統帳號或登入驗證；多人共用同一台機器時，可用它辨識及篩選各自的 Job。

`jobs` 會列出已 submit 的 Job，包含 `job_id`、owner、名稱與狀態。

`stoker update` 會顯示可更新的版本，且只有在明確輸入 `y` 或 `yes` 後才會更新。

若要安裝指定版本，請使用 `cargo install stoker-engine --version <VERSION> --force`。

`stoker uninstall` 也需要明確確認。請先停止 scheduler；你的 Job 資料與 logs 會保留在 Stoker 資料夾（預設為 `~/.stoker`）。

## 邊界與限制

- 只支援單機 queue；不做多機、遠端執行、分散式訓練、GPU 數量或容器排程。
- 必須在 Git repository 中，且工作目錄沒有未提交變更；沒有 Git 或不能 commit 就不能提交 Job。
- stoker 不管理 Python/Conda/CUDA 等環境，也不管理資料集、checkpoint、artifact 或實驗指標。
- 不提供 stoker 帳號、登入、權限控制；`--user` 只用於辨識與篩選。
