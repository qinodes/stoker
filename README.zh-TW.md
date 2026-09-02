# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)]
[![Version](https://img.shields.io/badge/version-0.5.2-informational)](Cargo.toml)

[English](README.md) | [日本語](README.ja.md) | 繁體中文

**stoker 是單機、本機 command 的任務排程器。**
每個 Job 都會在建立時所在的資料夾執行；`.stoker/runs` 只保存 logs。環境與產物由使用者自行管理。

## 安裝

請從 [GitHub Releases](https://github.com/qinodes/stoker/releases) 下載符合平台的壓縮檔，解壓縮 `stoker` 執行檔後加入 `PATH`：

- Windows：`stoker-windows-x86_64.zip`
- Linux：`stoker-linux-x86_64.tar.gz`
- macOS Apple Silicon：`stoker-macos-arm64.tar.gz`
- macOS Intel：`stoker-macos-x86_64.tar.gz`

每個 release 也會提供平台 binary 與 `SHA256SUMS`。Rust 開發者仍可使用 Cargo 安裝。

## 快速開始

需要 Rust，以及一個要執行 command 的目錄：

```bash
# 若已安裝 Cargo，也可以從 crates.io 安裝
cargo install stoker-engine

# 啟動背景 scheduler（Linux 與 Windows 皆適用）
stoker serve

# 在目前目錄建立 DRAFT Job
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# 確認內容後加入 queue（<JOB_ID> 由上一個指令輸出）
stoker show <JOB_ID>
stoker commit <JOB_ID>

# 查詢與管理
stoker status
stoker jobs
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
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

`jobs` 顯示 queue 摘要：`queue_order`、`job_id`、owner、名稱、狀態與時間。QUEUED Job 依 `queue_order` 排序，其他狀態顯示 `-`。可用 `--state draft` 或 `--state queued` 篩選。

要查看 Job 的完整資料、command 與執行路徑，請使用 `stoker show <JOB_ID>`。

## Job 狀態與取消

| 狀態 | 說明 |
| --- | --- |
| `DRAFT` | 已 submit，但尚未 commit 到 queue。 |
| `QUEUED` | 已 commit，正在等待執行。 |
| `STARTING` | scheduler 已取出 Job，正在準備來源目錄與程序。 |
| `RUNNING` | Job 的程序正在執行。 |
| `CANCELLING` | 已要求取消，stoker 正在停止程序並清理。 |
| `SUCCEEDED` | Job 已成功完成。 |
| `FAILED` | Job 程序失敗，或 stoker 無法完成執行流程。 |
| `CANCELLED` | Job 已被取消。 |
| `LOST` | scheduler 重啟時，發現先前執行中的 Job 已失去管理。 |

使用 `stoker cancel <JOB_ID>` 可以在 Job 執行前取消 `DRAFT` 或 `QUEUED` Job，也可以停止 `STARTING` 或 `RUNNING` Job。取消執行中的 Job 時，狀態會先變為 `CANCELLING`；stoker 會終止 Job 的程序樹、等待清理完成，再記錄為 `CANCELLED`。`cancel` 需要 scheduler 正在執行。

`stoker stop` 會停止 scheduler 並取消當前執行中的 Job；仍是 `QUEUED` 的 Job 會保留，等下次啟動 scheduler 後再處理。

Job 使用執行開始時來源目錄的內容；command 對來源目錄的檔案變更會保留。stoker 不會自動修改或還原目錄中的檔案。

`stoker update` 會從 GitHub Releases 檢查版本、使用 `SHA256SUMS` 驗證下載的 binary，且只有在明確輸入 `y` 或 `yes` 後才會更新。更新前請先停止 scheduler。

若不使用 Cargo，要安裝指定版本請從該版本的 GitHub Release 下載符合平台的 asset。Rust 開發者可使用 `cargo install stoker-engine --version <VERSION> --force`。

`stoker uninstall` 也需要明確確認。請先停止 scheduler；你的 Job 資料與 logs 會保留在 Stoker 資料夾（預設為 `~/.stoker`）。

## 邊界與限制

- 只支援單機 queue；不做多機、遠端執行、分散式訓練、GPU 數量或容器排程。
- submit 時的工作目錄必須存在且是目錄；不檢查目錄中的檔案變更。
- stoker 不管理 Python/Conda/CUDA 等環境，也不管理資料集、checkpoint、artifact 或實驗指標。
- 不提供 stoker 帳號、登入、權限控制；`--user` 只用於辨識與篩選。
