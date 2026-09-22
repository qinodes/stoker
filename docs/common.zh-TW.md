# 共用設定與維護指南

本指南適用於 `serial` mode 與 `scheduled` mode。內容包含時區、設定 snapshot、Log、執行 policy、Web UI、資料庫、更新與解除安裝。Job 或 Flow 的執行方式，請分別參考 [serial 詳細操作指南](serial.zh-TW.md) 與 [scheduled 詳細操作指南](scheduled.zh-TW.md)。


## Web UI

```bash
stoker ui start --open
stoker ui status
stoker ui stop
```

Web UI 預設只監聽 `127.0.0.1:8765`。若要讓區域網路上的裝置存取，請明確設定非 loopback 位址，且只用於信任的網路：

```bash
stoker ui start --host 0.0.0.0 --port 8765
```

## 時區與設定 snapshot

時間在 SQLite 中一律以 UTC 保存；`stoker jobs` 與 `stoker show` 顯示時才轉換。初始化時，Stoker 會將偵測到的 IANA timezone 寫入 `~/.stoker/config.json`。

```bash
stoker config show
stoker config set timezone Asia/Taipei
stoker config get timezone
stoker config unset timezone
stoker config set timezone       # 開啟互動式選擇器
stoker config snapshot
stoker config restore
```

設定的解析優先順序為 CLI option、`config.json`、作業系統 timezone。可用 `--timezone` 或 `--tz` 僅覆寫一次顯示：

```bash
stoker jobs --tz Asia/Tokyo
stoker show <JOB_ID> --timezone UTC
```

設定建立或更新時，snapshot 會保存在 `~/.stoker/snapshot/`；`config snapshot` 即使設定未變也會新建一份。

## Log 與執行 policy

預設會限制單一 Job log 至 64 MB、已結束 Job 的總 log 至 1024 MB，並保留最新 100 個 terminal Job 的 log。當可用磁碟空間低於 512 MB 時，scheduler 不會啟動下一個 Job。

修改 policy 前，先鎖定 queue，且不能有 `STARTING`、`RUNNING` 或 `CANCELLING` Job：

```bash
stoker queue lock
stoker policy set log-max-bytes-per-job 256
stoker policy set log-retention-jobs 30
stoker policy set termination-grace-ms 30000
stoker policy set max-runtime-ms 43200000
stoker policy unset max-runtime-ms
stoker policy show
stoker queue unlock
```

log 容量以不帶單位的整數 MB 輸入，例如 `256`。可用 `stoker policy get <KEY>` 查看單一有效值；log 達到上限或寫入失敗時，Stoker 仍會讀取子程序輸出，但較舊的分段可能被捨棄。


## 資料庫、更新與解除安裝

```bash
stoker db check
stoker db check --integrity
stoker db backup
stoker db backup <BACKUP_PATH>

# 還原前必須停止 scheduler。
stoker db restore <BACKUP_PATH> --yes

stoker --version
stoker update
stoker uninstall
```

未指定路徑時，`db backup` 會將帶時間戳的備份寫入 `<STOKER_HOME>/backups/`（通常是 `~/.stoker/backups/`），且包含 SQLite WAL。還原會取代目前資料庫；若 scheduler 中斷，執行中的 Job 會標示為 `LOST` 並鎖住 queue，請先人工處理再執行 `stoker queue unlock`。

更新或解除安裝前應停止 scheduler；兩者可加 `--yes` 略過確認。解除安裝不會刪除 Job 資料與 log。若使用 Cargo 安裝指定版本：

```bash
cargo install stoker-engine --version <VERSION> --force
```
