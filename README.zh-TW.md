<h1 align="center">
  <img src="assets/logo.svg" width="300" alt="stoker">
</h1>
<br>

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
![Version](https://img.shields.io/badge/version-1.0.2-informational)

[English](README.md) | [日本語](README.ja.md) | 繁體中文

**stoker 是一個輕量、跨平台、本機 command(CLI) 的任務排程器。**
每個 Job 都會自動在建立時所在的資料夾執行。

## 安裝

### 未安裝cargo

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

### 已安裝cargo

```bash
cargo install stoker-engine
```

## 快速開始

```bash

# 啟動背景 scheduler（Linux, Macos和 Windows 皆適用）
stoker start

# 在目標目錄建立 DRAFT Job
stoker add --user <任意使用者名稱> --name <job名稱> --cmd <待執行指令>
# 例如: 
# stoker add --user alice --name exp-a --cmd "python train.py --lr 0.0001"

# 確認內容後加入 queue（<JOB_ID> 由上一個指令輸出）
# 查看Job設定細節
stoker show <JOB_ID>
# 送出Job(draft->queued)
stoker commit <JOB_ID>
# 將所有 DRAFT Job 依建立時間加入 queue
stoker commit --all

# 暫時保留所有 queued Job，之後再恢復
stoker pause
stoker resume

# 重新排序 queued Job 前先鎖定 queue，完成後明確解除鎖定
stoker lock-queue
stoker queue edit
stoker status
stoker unlock-queue

# 查詢與管理

# 查看 stoker server(scheduler) 狀態
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

# 取消指定Job（DRAFT、QUEUED、PAUSED、STARTING、RUNNING、CANCELLING 都可以取消）
stoker cancel <JOB_ID>
# 在腳本中可加上 --yes 略過確認提示。

# 停止server(scheduler)
# 若有正在執行的 Job，stoker 會先詢問是否強制取消
# `QUEUED` 與 `PAUSED` Job 會保留，等下次啟動 scheduler 後再處理
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

`--user` 是 stoker 的邏輯 owner 標籤，不是作業系統帳號或登入驗證；

## Queue 鎖定與編輯器

編輯 queue 前先執行 `stoker lock-queue`，準備讓 scheduler 繼續接收工作時再執行 `stoker unlock-queue`。鎖定狀態會持久保存：CLI 結束、scheduler 停止或重新啟動後仍然有效。鎖定時 scheduler 不會再 claim 其他 `QUEUED` Job。已經處於 `STARTING`、`RUNNING` 或 `CANCELLING` 的 Job 不會被停止，仍可正常完成。

`stoker status` 一律顯示 `Queue: locked` 或 `Queue: unlocked`。鎖定時也會提示 scheduler 不會再啟動下一個 queued Job。Queue 鎖定時，`stoker commit`、`stoker commit --all`、`stoker pause` 與 `stoker resume` 都會被拒絕；`stoker cancel` 仍可使用，`stoker add` 也仍可使用，因為它建立的是 `DRAFT` Job。

`stoker queue edit` 必須在 queue 已鎖定時使用。空 queue 仍可成功鎖定，並顯示 `Queue locked. No queued jobs to reorder.`；編輯空 queue 會成功顯示 `No queued jobs to reorder.`，不會開啟空白的終端機編輯器。離開編輯器不會解除 queue 鎖定。

編輯器只顯示依執行順序排列的 `QUEUED` Job：

| 模式 | 按鍵 | 動作 |
| --- | --- | --- |
| 瀏覽 | `↑` / `↓` | 選取 Job。 |
| 瀏覽 | `Enter` | 對選取的 Job 進入移動模式。 |
| 瀏覽 | `q` | 離開編輯器並保持 queue 鎖定。 |
| 移動 | `↑` / `↓` | 調整選取 Job 的位置。 |
| 移動 | `Enter` | 保留移動結果並返回瀏覽模式。 |
| 移動 | `q` | 只復原目前這次移動，並返回瀏覽模式。 |

如果另一個終端機在編輯期間取消了選取的 Job，下一次移動或復原不會把它恢復；編輯器會提示 Job 已移除並重新載入 queued 清單。如果被取消的是其他 queued Job，編輯器會在下一次移動時使用目前的順序，並保留該取消結果。

## Job 狀態與取消

| 狀態 | 說明 |
| --- | --- |
| `DRAFT` | 已 add，但尚未 commit 到 queue。 |
| `QUEUED` | 已 commit，正在等待執行。 |
| `PAUSED` | 暫時保留；以 `stoker resume` 放回 queue。 |
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
