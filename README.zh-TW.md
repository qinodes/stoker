<h1 align="center">
  <img src="assets/logo.svg" width="300" alt="stoker">
</h1>
<br>

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
![Version](https://img.shields.io/badge/version-1.2.1-informational)

[English](README.md) | [日本語](README.ja.md) | 繁體中文

**stoker 是一個輕量、跨平台、用來排程多項耗時工作的 CLI。**

特別為了能依序穩定地執行多個耗時任務而設計。

- 適合多人輪流共用、但想避免任務互相搶占資源的需求。

- 支援多人同時下達排程任務。

- 每個 Job 默認都會從你提交指令時(`stoker add`)所在的資料夾執行。

## 安裝

### 不使用 Cargo

從 [GitHub Releases](https://github.com/qinodes/stoker/releases) 下載符合平台的壓縮檔，解壓縮 `stoker` 執行檔後加入 `PATH`：

- Windows：`stoker-windows-x86_64.zip`
- Linux：`stoker-linux-x86_64.tar.gz`
- macOS Apple Silicon：`stoker-macos-arm64.tar.gz`

下載的執行檔不會自動加入環境變數，請將執行檔所在的資料夾加入 `PATH`：

**Windows：** 在「環境變數」的「使用者變數」中編輯 `Path`，新增執行檔所在的資料夾，然後重新開啟終端機。

**macOS／Linux：** 將以下內容加入 `~/.zshrc`（macOS）或 `~/.bashrc`（Linux），把 `/path/to/stoker` 換成執行檔所在的實際資料夾，然後重新開啟終端機：

```bash
export PATH="/path/to/stoker:$PATH"
```

寫入後若要讓目前的終端機立即套用設定，可執行：

```bash
source ~/.bashrc  # Linux
source ~/.zshrc   # macOS
```


每個 release 也會提供平台 binary 與 `SHA256SUMS`。

### 使用 Cargo

```bash
cargo install stoker-engine
```

## 快速開始

### 基本流程

```bash
# 啟動背景 scheduler
stoker start

# 在任務執行需要的根目錄下，提交一個 Job
stoker add --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# 將 DRAFT Job 加入 queue
stoker commit <JOB_ID>

# 查看 scheduler 與 Job 狀態
stoker status
```

`stoker add` 就是提交 Job 的步驟，會先建立一個 DRAFT Job。使用指令輸出的
`JOB_ID` 執行 `stoker commit`，即可將它加入 queue。Job 會依 queue 順序一次執行一個。

### 指令參考

```bash

# 啟動背景 scheduler（Linux、macOS 與 Windows 皆適用）
stoker start

# 在目標目錄建立 DRAFT Job
stoker add --user <任意使用者名稱> --name <job名稱> --cmd "<待執行指令>"
# 例如: 
# stoker add --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# 確認內容後加入 queue（<JOB_ID> 由上一個指令輸出）
# 查看Job設定細節
stoker show <JOB_ID>
# 送出Job(draft->queued)
stoker commit <JOB_ID>
# 將所有 DRAFT Job 依建立時間加入 queue
stoker commit --all

# 重新排序 queued Job 前先鎖定 queue，完成後明確解除鎖定
stoker queue lock
stoker queue edit
stoker status
stoker queue unlock

# 查詢與管理

# 查看 scheduler 狀態
stoker status

# 查看所有Job狀態
stoker jobs

# 查看指定Job狀態(篩選)
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
# 組合篩選條件
stoker jobs --user alice --state failed

# 清除 SUCCEEDED、FAILED、CANCELLED、LOST Job 與其 logs
# scheduler 執行中也可以使用
stoker clean

# 查看目前已有的日誌，輸出完即結束。
stoker logs <JOB_ID>

# 持續顯示該 Job 新產生的 log，直到 Job 結束或你按 Ctrl+C
stoker logs -f <JOB_ID>

# 取消指定Job（DRAFT、QUEUED、STARTING、RUNNING、CANCELLING 都可以取消）
stoker cancel <JOB_ID>
# 在腳本中可加上 --yes 略過確認提示。

# 停止server(scheduler)
# 若有正在執行的 Job，stoker 會先詢問是否強制取消
# `QUEUED` Job 會保留，等下次啟動 scheduler 後再處理
stoker stop
# 在腳本中可加上 --yes 略過確認提示。

# 查看當前版本
stoker --version

# 更新到最新版
# 更新前要先停止 scheduler
stoker update
# 在腳本中可加上 --yes 略過確認提示。

# 解除安裝
# 解除前要先停止 scheduler
# Job 資料與 logs 會保留在 Stoker 資料夾（預設為 macOS／Linux 的 `~/.stoker`、Windows 的 `%USERPROFILE%\.stoker`）。
stoker uninstall
# 在腳本中可加上 --yes 略過確認提示。
```

`--cmd` 後面的完整指令必須用引號包住。

Job 會在背景執行，不具備互動式終端機。請使用非互動式指令與參數。

指令會交由平台的 shell 執行：Linux／macOS 使用 `sh`，Windows 使用
`cmd.exe`，因此 shell 語法與可用程式可能因平台不同。

`--user` 是 stoker 的邏輯 owner 標籤，不是作業系統帳號或登入驗證；

## 時區設定

SQLite 內的時間一律以 UTC 保存；`stoker jobs` 與 `stoker show` 顯示時，才依照顯示時區轉換，並保留 RFC3339 offset。

首次初始化 Stoker 資料夾時，會偵測作業系統的 IANA 時區並寫入：

```text
~/.stoker/config.json
```

設定內容例如：

```json
{
  "timezone": "Asia/Taipei"
}
```

設定或查詢時區：

```bash
stoker config show
stoker config set timezone Asia/Taipei
stoker config get timezone
stoker config unset timezone
```

`stoker config show` 會顯示 config 檔案位置與完整的已儲存設定。若要查看實際生效的時區與來源，請使用 `stoker status`。

省略時區值時，Stoker 會開啟互動式選擇器：

```bash
stoker config set timezone
```

當 Stoker 建立或更新 `config.json` 時，會在下列位置保留帶有時間戳的 snapshot：

```text
~/.stoker/snapshot/
```

如果要在進行高風險變更前主動建立 snapshot，可以執行：

```bash
stoker config snapshot
```

即使設定內容沒有變更，這個指令仍然會建立一份新的 snapshot。

使用互動式還原畫面選擇 snapshot：

```bash
# 最新的 snapshot 會排在最上方；使用方向鍵選擇、按 `Enter` 查看唯讀詳細內容、按 `Esc` 或 `q` 回到清單，最後按 `Enter` 再按 `y` 確認還原
# 還原前會先把目前的設定保存成新的 snapshot。
stoker config restore
```

單次指令可以用 `--timezone` 或較短的 `--tz` 覆寫設定：

```bash
stoker jobs --tz Asia/Tokyo
stoker show <JOB_ID> --timezone UTC
```

解析優先順序為 CLI 參數、`config.json`、作業系統時區。

## Queue 鎖定與編輯器

可以透過`stoker status` 確認Queue狀態。

修改 queue 前先執行 `stoker queue lock`，完成修改後執行 `stoker queue unlock`。

鎖定時不能執行 `stoker commit` 或 `stoker commit --all`，但仍可 `cancel` 或 `add`。

`stoker queue edit` 必須在鎖定後使用。

編輯器只顯示依執行順序排列的 `QUEUED` Job：

| 模式 | 按鍵 | 動作 |
| --- | --- | --- |
| 瀏覽 | `↑` / `↓` | 選取 Job。 |
| 瀏覽 | `Enter` | 對選取的 Job 進入移動模式。 |
| 瀏覽 | `q` / `Esc` | 離開編輯器並保持 queue 鎖定。 |
| 移動 | `↑` / `↓` | 調整選取 Job 的位置。 |
| 移動 | `Enter` | 保留移動結果並返回瀏覽模式。 |
| 移動 | `q` / `Esc` | 只復原目前這次移動，並返回瀏覽模式。 |


## Job 狀態與取消

| 狀態 | 說明 |
| --- | --- |
| `DRAFT` | 已 add，但尚未 commit 到 queue。 |
| `QUEUED` | 已 commit，正在等待執行。 |
| `STARTING` | scheduler 已取出 Job，正在準備來源目錄與程序。 |
| `RUNNING` | Job 的程序正在執行。 |
| `CANCELLING` | 已要求取消，stoker 正在停止程序並清理。 |
| `SUCCEEDED` | Job 已成功完成。 |
| `FAILED` | Job 程序失敗，或 stoker 無法完成執行流程。 |
| `CANCELLED` | Job 已被取消。 |
| `LOST` | scheduler 重啟時，發現先前執行中的 Job 已失去管理。 |


## 補充說明

command 對來源目錄的檔案變更會保留。stoker 不會自動修改或還原目錄中的檔案。

安裝指定版本:

`cargo install stoker-engine --version <VERSION> --force`。

logs 保存於 `.stoker/runs/<JOB_ID>/stdout.log` 與 `.stoker/runs/<JOB_ID>/stderr.log`。

## 邊界與限制

- 只支援單機 queue；不做多機、遠端執行、分散式訓練、GPU 數量或容器排程。
- add 時的工作目錄必須存在且是目錄；不檢查目錄中的檔案變更。
- stoker 不管理 Python/Conda/CUDA 等環境，也不管理資料集、checkpoint、artifact 或實驗指標。
- 不提供 stoker 帳號、登入、權限控制；`--user` 只用於辨識與篩選。
