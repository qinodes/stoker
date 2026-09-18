# Serial mode 詳細操作指南

本指南說明 `serial` mode：每個 Job 只執行一次，並依 queue 順序一次執行一個。從 [README.zh-TW](../README.zh-TW.md) 開始；需要排程或多 task 相依時，請改看 [scheduled 詳細操作指南](scheduled.zh-TW.md)。


## Job 的建立與提交

在 command 應執行的資料夾建立 DRAFT Job。`--cmd` 後的完整 command 必須用引號包住。

```bash
stoker create --user <USER> --name <NAME> --cmd "<COMMAND>"
stoker show <JOB_ID>

# 依輸入順序提交指定的 DRAFT Job。
stoker commit <JOB_ID> [<JOB_ID>...]

# 依建立時間提交全部或某位 user 的 DRAFT Job。
stoker commit --all
stoker commit --user <USER>
```

`--user` 是邏輯 owner 標籤，用於辨識與篩選，不是作業系統帳號或驗證機制。Job 會在背景執行，不具備互動式終端機；請使用非互動式指令與參數。Linux／macOS 以 `sh` 執行 command，Windows 以 `cmd.exe` 執行，因此 shell 語法與可用程式可能不同。

```bash
# 查看與篩選 Job。
stoker jobs
# stoker jobs [篩選條件]
stoker jobs --user alice
stoker jobs --state queued
stoker jobs --user alice --state failed

# 查看既有 log，或持續追蹤到 Job 結束。
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>

# 取消 DRAFT、QUEUED、STARTING、RUNNING 或 CANCELLING Job。
stoker cancel <JOB_ID>
```

## Scheduler 與 queue

```bash
stoker start
stoker status
stoker stop
```

`stoker stop` 若有 active Job 會詢問是否強制取消；使用 `--yes` 可略過確認。`QUEUED` Job 會保留到下次 scheduler 啟動。

重新排序前必須鎖定 queue；完成後再明確解除鎖定。鎖定期間不能 commit，但仍可 create 或 cancel。

```bash
stoker queue lock
stoker queue edit
stoker queue unlock
```

`queue edit` 只顯示 `QUEUED` Job。瀏覽模式用 `↑`／`↓` 選取、`Enter` 進入移動模式、`q` 或 `Esc` 離開並保持鎖定；移動模式用 `↑`／`↓` 調整位置、`Enter` 保留、`q` 或 `Esc` 復原目前這次移動。

| State | 意義 |
| --- | --- |
| `DRAFT` | 已建立，尚未提交至 queue。 |
| `QUEUED` | 已提交，等待執行。 |
| `STARTING` | scheduler 正在準備 Job。 |
| `RUNNING` | Job 程序正在執行。 |
| `CANCELLING` | 已要求取消，正在停止程序與清理。 |
| `SUCCEEDED` | Job 成功完成。 |
| `FAILED` | Job 程序失敗，或 stoker 無法完成執行。 |
| `CANCELLED` | Job 已取消。 |
| `LOST` | scheduler 重啟後失去原本執行中 Job 的管理。 |

用 `stoker clean` 移除 `SUCCEEDED`、`FAILED`、`CANCELLED` 與 `LOST` Job 及其 log；scheduler 執行中也可使用。

## Docker Job

若 Stoker 應等待 container 結束才處理下一個 Job，請以前景模式執行 Docker：

```bash
docker run <IMAGE> <COMMAND>
```

不要使用 `docker run -d`：container 啟動後 command 會立刻返回，Stoker 會視為 Job 已完成。
