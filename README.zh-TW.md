<h1 align="center">
  <img src="assets/logo.svg" width="300" alt="stoker">
</h1>
<br>

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
[![CI](https://github.com/qinodes/stoker/actions/workflows/ci.yml/badge.svg)](https://github.com/qinodes/stoker/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/qinodes/stoker/branch/main/graph/badge.svg)](https://codecov.io/gh/qinodes/stoker)
[![Crates.io](https://img.shields.io/crates/v/stoker-engine)](https://crates.io/crates/stoker-engine)
[![Downloads](https://img.shields.io/crates/d/stoker-engine)](https://crates.io/crates/stoker-engine)
[![License](https://img.shields.io/github/license/qinodes/stoker)](https://github.com/qinodes/stoker/blob/main/LICENSE)

[English](README.md) | 繁體中文 | [日本語](README.ja.md)

**stoker 是一個以 Rust 撰寫、低資源占用的輕量任務排程 CLI。** 提供兩種運作模式：serial mode 與 scheduled mode。

serial mode 適合執行**長時間運作、需要大量 GPU、CPU 或記憶體**的 Job，並且一次只執行一個。

scheduled mode 適合**定期執行**、具相依關係的**輕量**多步驟 task，可在一個 Flow 中執行多個 task。

Job 狀態與 log 都保存在本機，不需要架設外部資料庫。

<p align="center">
  <img src="assets/ui-demo-v2.png" alt="Stoker Web UI 示意圖">
</p>

## 安裝

推薦的安裝程式會下載最新版 release、驗證 SHA256、安裝給目前使用者，並更新使用者層級的 `PATH`；不需要管理員權限。

**Windows PowerShell**

```powershell
irm https://github.com/qinodes/stoker/releases/latest/download/stoker-install.ps1 | iex
```

**Linux／macOS**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/qinodes/stoker/releases/latest/download/stoker-install.sh | sh
```

也可以使用 Cargo：

```bash
cargo install stoker-engine
```

手動安裝與指定版本請看 [GitHub Releases](https://github.com/qinodes/stoker/releases)。

共用設定、Web UI、時區、log、policy、備份、更新與解除安裝，請看 [共用設定與維護指南](docs/common.zh-TW.md)。

## 1. serial mode

**serial mode** 適合每個 Job 只執行一次、依次順序處理的情境。請在 command 應該執行的資料夾內使用 `stoker create`。

基本格式：

```text
stoker create --user <USER> --name <NAME> --cmd "<COMMAND>"
stoker show <JOB_ID>
stoker commit <JOB_ID>
```

```bash
# 切換到 serial mode。
stoker queue lock
stoker mode set serial
stoker queue unlock

# 在共用機器上啟動一次背景 scheduler。
stoker start

# 建立 DRAFT Job；command 之後會從目前資料夾執行。
stoker create --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# <JOB_ID> 是上一個指令輸出的 Job UUID；確認後加入 queue。
stoker show <JOB_ID>
stoker commit <JOB_ID>

# 查看 queue 與這個 Job 的輸出。
stoker jobs
stoker logs -f <JOB_ID>
```

commit 後，Job 會一次執行一個。`--user` 是用來顯示與篩選的 owner 標籤，不是作業系統帳號或驗證機制。

queue 編輯、取消與所有 serial 指令，請看 [serial 詳細操作指南](docs/serial.zh-TW.md)。

## 2. scheduled mode

重複工作或有相依關係的多個 task，請使用 **scheduled mode**。先完成 task 與 schedule 的設定，再 commit Flow。

基本格式：

```bash
# --once-at、--daily、--every 三者必須且只能選一個。
# --schedule-timezone 不是必要參數，只能和 --daily 一起使用。
# --first-at 不是必要參數，只能和 --every 一起使用。
stoker flow create <FLOW_ID> --user <USER> --name <NAME> (--once-at <RFC3339> | --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] | --every <Nm|Nh> [--first-at <RFC3339>])
stoker flow task add <FLOW_ID> <TASK_ID> --name <NAME> --cmd "<COMMAND>"
stoker flow commit <FLOW_ID>
```

```bash
# 切換到 scheduled mode。
stoker queue lock
stoker mode set scheduled
stoker queue unlock

stoker start

# 建立 DRAFT Flow。
# frequent_a001 是 <FLOW_ID>；之後的 flow 指令都使用 frequent_a001。
# --every 15m 表示每 15 分鐘一次；--first-at 指定第一次執行時間。
stoker flow create frequent_a001 --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00

# 也可以改用一次性或每日 schedule：
# stoker flow create my_task_once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
# stoker flow create my_task_nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# 切換到 task 執行時要使用的資料夾下，將 task 加入 frequent_a001 Flow。
# refresh、publish 是 <TASK_ID>；--after refresh 引用前一個 task 的 ID。
stoker flow task add frequent_a001 refresh --name refresh --cmd "python refresh_cache.py"
stoker flow task add frequent_a001 publish --name publish --cmd "python publish_summary.py" --after refresh

# 驗證並啟用 Flow。
stoker flow commit frequent_a001
stoker flow list
```

### 宣告式 Flow 定義

若要把整組 Flow 定義放入版本控制或 code review，可使用 JSON source mode：

```bash
stoker flow export --dir ./flow-definitions
# 編輯剛匯出的 JSON。
stoker queue lock
stoker flow source-mode sync
stoker flow sync <EXPORTED_JSON> --dry-run
stoker flow sync <EXPORTED_JSON>
# 確認結果後才恢復排程。
stoker queue unlock
```


一次性／週期性排程、standalone scheduled Job、重試、Flow 編輯、查看 run、取消與 recovery，請看 [scheduled 詳細操作指南](docs/scheduled.zh-TW.md)。
