# Flow 與 scheduled mode 詳細操作指南

本指南說明 `scheduled` mode 的 Flow 與 standalone scheduled Job。從 [README.zh-TW](../README.zh-TW.md) 開始。單次、依序執行的工作請看 [serial 詳細操作指南](serial.zh-TW.md)。


## 切換 mode

Flow 只存在於 `scheduled` mode。切換前必須先鎖定 queue；

```bash
stoker mode show
stoker queue lock
stoker mode set scheduled
stoker queue unlock
```

有 execution 正在啟動、執行、取消、清理或 recovery 時，不能切換 mode。

## 建立與執行 Flow

Flow 將多個 task 組成一個 run。建立與啟用 Flow 分成三步：先建立包含 schedule 的 draft，再新增 task，最後 commit Flow。

### 基本格式

#### 先建立 Flow，請從以下 schedule 任選一個：

```bash
# <FLOW_ID> 是 `flow create` 後面的第一個名稱。
# --name 是給人看的顯示名稱，可以和 ID 不同。
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --once-at <RFC3339>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm> --schedule-timezone <IANA_ZONE>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh> --first-at <RFC3339>
```


#### 將 task 加入 Flow 中，新增 task 的常用格式：

(先切換到 task 要執行的資料夾，再執行 `flow task add`；目前資料夾會作為 task 的工作目錄。)

```bash
# <TASK_ID> 是 flow task add 後面的 task 識別名稱。
# --name 是給人看的顯示名稱，可以和 ID 不同。
# --after <TASK_ID> 要求上游 task 成功
# --after-failure <TASK_ID> 要求上游 task 失敗。
# 需要多個相依條件時，重複加入 --after 或 --after-failure。
# 加上 --match any 可改為任一條件符合；
# --retries <N> 可設定 task 失敗後的重試次數，0 表示不重試。
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>"
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <TASK_ID>
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <UPSTREAM_TASK_ID_1> --after <UPSTREAM_TASK_ID_2> --match all --retries 1
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after-failure <TASK_ID>
```

#### 完成 task 後，使用以下指令啟用 Flow：

```bash
stoker flow commit <FLOW_ID>
```

例如：

```bash
# 沒有上游 task。
stoker flow task add nightly refresh --name refresh --cmd "python refresh_cache.py"

# 等待一個上游 task 成功。
stoker flow task add nightly publish --name publish --cmd "python publish_summary.py" --after refresh

# 等待兩個上游 task 都成功，失敗時最多重試一次。
stoker flow task add nightly notify --name notify --cmd "python send_notification.py" --after refresh --after publish --match all --retries 1
```

### 完整範例

```bash
# 建立 DRAFT Flow；nightly 是 <FLOW_ID>，也是顯示名稱。
stoker flow create nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# 在 task 要執行的資料夾新增 task；
# 沒有上游 task。
stoker flow task add nightly refresh --name refresh --cmd "python refresh_cache.py"
# 等待一個上游 task 成功。
stoker flow task add nightly publish --name publish --cmd "python publish_summary.py" --after refresh

# 驗證並啟用 Flow。
stoker flow commit nightly
```

若要使用其他 schedule，只替換第一步的 schedule 參數：

```bash
stoker flow create once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
stoker flow create frequent --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00
```

### 查看、手動執行與 logs

```bash
stoker flow list
stoker flow list --user <USER>
stoker flow show <FLOW_ID>
stoker flow runs <FLOW_ID>
stoker flow occurrences <FLOW_ID>

# flow run 會立即建立一次 manual run
# 同一 --request-id 可安全重送。
# --replace-next 會在 manual run 開始後取代一個明確的未來 occurrence。
stoker flow run <FLOW_ID>
stoker flow run <FLOW_ID> --request-id <UUID>
stoker flow run <FLOW_ID> --replace-next --request-id <UUID>

stoker flow show <FLOW_ID> --run <RUN_ID>
stoker flow show <FLOW_ID> --run <RUN_ID> --task <TASK_ID>

# --attempt <N> 從 1 起算；省略時顯示該 task 的全部 attempts。
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --follow
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N> --follow

stoker flow cancel <FLOW_ID> --run <RUN_ID>
stoker flow cancel <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
```


### 範例

```bash
# 查看 Flow、run 與 scheduled occurrence。
stoker flow show nightly
stoker flow runs nightly
stoker flow occurrences nightly

# 立即建立 manual run。
stoker flow run nightly --request-id <UUID>
stoker flow run nightly --replace-next --request-id <UUID>

# 查詢、追蹤或取消 run 中的 task。
stoker flow show nightly --run <RUN_ID> --task refresh
stoker flow logs nightly --run <RUN_ID> --task refresh --attempt 1 --follow
stoker flow cancel nightly --run <RUN_ID>
stoker flow cancel nightly --run <RUN_ID> --task refresh
```

## 編輯已提交的 Flow

先 freeze，再修改 future draft，最後 apply。`--revision N` 是 draft 的 compare-and-swap revision；revision 不符時不會套用修改。

```bash
stoker flow edit begin nightly
stoker flow task update nightly publish --retries 2 --revision 0
stoker flow schedule set nightly --daily 01:00 --schedule-timezone Asia/Tokyo --revision 1
stoker flow edit apply nightly --revision 2
```

以下是各種修改操作的格式。`--revision <N>` 可加在任何修改指令最後，用來確認 draft revision：

```text
stoker flow task update <FLOW_ID> <TASK_ID> --cmd <COMMAND>
stoker flow task update <FLOW_ID> <TASK_ID> --cwd <DIR>
stoker flow task update <FLOW_ID> <TASK_ID> --retries <N>
stoker flow task update <FLOW_ID> <TASK_ID> --after <TASK_ID>
stoker flow task update <FLOW_ID> <TASK_ID> --after-failure <TASK_ID>
stoker flow task update <FLOW_ID> <TASK_ID> --match all
stoker flow task update <FLOW_ID> <TASK_ID> --match any
stoker flow task update <FLOW_ID> <TASK_ID> --clear-dependencies

stoker flow task remove <FLOW_ID> <TASK_ID>
stoker flow task remove <FLOW_ID> <TASK_ID> --scope future
stoker flow task remove <FLOW_ID> <TASK_ID> --scope current --run <RUN_ID>
stoker flow task remove <FLOW_ID> <TASK_ID> --scope both --run <RUN_ID>

stoker flow schedule set <FLOW_ID> --once-at <RFC3339>
stoker flow schedule set <FLOW_ID> --daily <HH:mm>
stoker flow schedule set <FLOW_ID> --daily <HH:mm> --schedule-timezone <IANA_ZONE>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --schedule-timezone <IANA_ZONE>
```

`once`、`daily`、`every` 三種 schedule 不能互換，只能修改同類 schedule。

`flow edit discard <FLOW_ID> --revision <N>` 丟棄 draft，但 Flow 仍保持 frozen；

執行 `flow edit apply` 才會解除 freeze。

`task remove` 預設作用於 `future`；

`current` 或 `both` 需要 `--run`，且 Flow 必須已 freeze。

```bash
stoker flow disable nightly
stoker flow enable nightly
stoker request show <REQUEST_ID>
```

disable 只停止未來 automatic trigger，不阻止合法的 manual run。

## Standalone scheduled Job

在 `scheduled` mode，`stoker add` 也可建立單一 command 的 scheduled Job。建立後仍需 `commit` 才會啟用。

```bash
stoker add --user alice --name frequent --cmd "python refresh.py" --every 15m --first-at 2099-01-01T10:00:00+09:00
stoker commit <JOB_ID>

stoker runs <JOB_ID>
stoker occurrences <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
stoker logs <JOB_ID> --run <RUN_ID> --follow
stoker cancel <JOB_ID> --run <RUN_ID>
```

建立時的三種形式：

```text
stoker add --user <USER> --name <NAME> --cmd <COMMAND> --once-at <RFC3339> [--retry <N>]
stoker add --user <USER> --name <NAME> --cmd <COMMAND> --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] [--retry <N>]
stoker add --user <USER> --name <NAME> --cmd <COMMAND> --every <Nm|Nh> [--first-at <RFC3339>] [--retry <N>]
```

`--retry` 是 standalone Job 的重試次數（Flow task 使用 `--retries`）。若要修改已 commit 的 schedule，先 freeze、修改、再 unfreeze：

```bash
stoker freeze <JOB_ID>
stoker schedule set <JOB_ID> --every 2h --expected-draft-revision <N>
stoker unfreeze <JOB_ID> --expected-draft-revision <N>
```

`stoker draft discard <JOB_ID> --expected-draft-revision <N>` 只丟棄 draft，Job 仍保持 frozen。`stoker disable <JOB_ID>` 與 `stoker enable <JOB_ID>` 控制 automatic trigger；`run --skip-next` 在 manual run 開始後取代一個未來 occurrence。

## 排程規則與 recovery

One-time 時間必須是包含秒數與明確 UTC offset 的 RFC 3339，例如 `2099-01-01T23:30:00+09:00`。Daily 使用 `HH:mm` 與 IANA timezone；錯過的 daily occurrence 不補跑，DST 不存在的時間會跳過，重複時間採用較早的 instant。

`--every` 只接受小寫整數分鐘或小時，例如 `1m`、`15m`、`1h`、`2h`；最小值分別是 1 分鐘與 1 小時。未指定 `--first-at` 時，第一次執行在 commit 或套用排程後的一個完整週期；指定時必須是未來的 RFC 3339 時間，之後會以 UTC 經過時間推進且不隨 DST 偏移。停機期間錯過的週期不補跑。

若重啟後 run 留在 `RECOVERING`，確認程序已停止後再 reconcile；所有 recovery 完成後才能解除 queue lock：

```bash
stoker recovery reconcile <RUN_ID> --confirm-stopped
stoker queue unlock
```

所有 Flow 指令都以 `stoker flow` 開頭；頂層 scheduled-job 指令只接受 standalone Job UUID。需要 option 的完整語法時，使用 `stoker flow --help`、`stoker flow task --help` 或各子指令的 `--help`。
