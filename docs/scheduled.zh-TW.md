# Flow 與 scheduled mode 詳細操作指南

本指南說明如何在 `scheduled` mode 使用兩種工作方式：

- **Flow**：把多個 task 組成一個工作流程。
- **standalone scheduled Job**：只執行一個 command，但可以依排程重複執行。

如果你是第一次使用 stoker，建議先看 [README.zh-TW](../README.zh-TW.md)。
單次、依序執行的工作請看 [serial 詳細操作指南](serial.zh-TW.md)。

如果只想先建立第一個 Flow，請依序閱讀「切換到 scheduled mode」、「建立並執行第一個 Flow」和「查看、手動執行與 logs」。後面的章節再說明編輯、JSON source mode 與 recovery。

## 先了解幾個名詞

- **Flow**：一組按照順序或相依關係執行的 task。
- **task**：Flow 中的一個 command。
- **run**：Flow 實際執行一次的結果。
- **occurrence**：排程產生的一次預定執行。
- **draft**：尚未套用的修改內容。
- **standalone scheduled Job**：不使用 Flow，只執行一個 command 的排程工作。

## 1. 切換到 scheduled mode

Flow 只能在 `scheduled` mode 中使用。切換前，請先鎖定 queue。

```bash
stoker mode show
stoker queue lock
stoker mode set scheduled
stoker queue unlock
```

這四個指令的作用如下：

1. `stoker mode show`：查看目前的 mode。
2. `stoker queue lock`：暫時鎖定 queue，避免切換期間有新的工作進入。
3. `stoker mode set scheduled`：切換到 `scheduled` mode。
4. `stoker queue unlock`：完成切換後解除 queue lock。

如果有 execution 正在啟動、執行、取消、清理或 recovery，就不能切換 mode。請等這些工作完成後再切換。

## 2. 建立並執行第一個 Flow

一個 Flow 的建立分成三步：

1. 建立包含 schedule 的 draft Flow。
2. 將 task 加入 Flow。
3. commit Flow，讓它開始依排程執行。

### 2.1 建立 Flow

`<FLOW_ID>` 是 Flow 的識別名稱。`--name` 是給人看的顯示名稱，兩者可以不同。

建立 Flow 時，請從以下三種 schedule 中選一種：

```bash
# 一次性執行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --once-at <RFC3339>

# 每天執行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm> --schedule-timezone <IANA_ZONE>

# 每隔一段時間執行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh> --first-at <RFC3339>
```

### 2.2 將 task 加入 Flow

請先切換到 task 要執行的資料夾，再執行 `flow task add`。目前資料夾會成為 task 的工作目錄。

`<TASK_ID>` 是 task 的識別名稱。`--name` 是給人看的顯示名稱，兩者可以不同。

```bash
# 不指定相依 task
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>"

# 需要一個上游 task 成功
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <TASK_ID>

# 需要兩個上游 task 都成功，失敗時最多重試一次
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <UPSTREAM_TASK_ID_1> --after <UPSTREAM_TASK_ID_2> --match all --retries 1

# 需要上游 task 失敗
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after-failure <TASK_ID>
```

依相依關係增加 task 時，請注意：

- `--after <TASK_ID>`：上游 task 必須成功。
- `--after-failure <TASK_ID>`：上游 task 必須失敗。
- 需要多個條件時，可以重複加入 `--after` 或 `--after-failure`。
- `--match all`：所有條件都符合時才執行。這是預設行為。
- `--match any`：任一條件符合時就執行。
- `--retries <N>`：設定 task 失敗後的重試次數。`0` 代表不重試。

### 2.3 Commit Flow

完成 task 後，使用以下指令驗證並啟用 Flow：

```bash
stoker flow commit <FLOW_ID>
```

### 2.4 完整範例

以下範例會建立一個每天晚上 23:30 執行的 Flow。`nightly` 同時是 `<FLOW_ID>` 和顯示名稱。

```bash
stoker flow create nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# 請在 task 要執行的資料夾中執行這些指令。
stoker flow task add nightly refresh --name refresh --cmd "python refresh_cache.py"
stoker flow task add nightly publish --name publish --cmd "python publish_summary.py" --after refresh

# 等待兩個上游 task 都成功，失敗時最多重試一次。
stoker flow task add nightly notify --name notify --cmd "python send_notification.py" --after refresh --after publish --match all --retries 1

stoker flow commit nightly
```

如果要使用其他 schedule，只需要替換建立 Flow 時的 schedule 參數：

```bash
stoker flow create once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
stoker flow create frequent --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00
```

### 2.5 排程規則

#### 一次性排程

`--once-at` 必須使用包含秒數和明確 UTC offset 的 RFC 3339 時間，例如：

```text
2099-01-01T23:30:00+09:00
```

#### 每日排程

`--daily` 使用 `HH:mm` 格式。需要指定時區時，再加上 `--schedule-timezone <IANA_ZONE>`。

- 錯過的 daily occurrence 不會補跑。
- DST 不存在的時間會被跳過。
- DST 重複的時間會使用較早的 instant。

#### 週期排程

`--every` 只接受小寫的整數分鐘或小時，例如 `1m`、`15m`、`1h` 和 `2h`。

- 分鐘的最小值是 `1m`。
- 小時的最小值是 `1h`。
- 沒有指定 `--first-at` 時，第一次執行會在 commit 或套用排程後的一個完整週期發生。
- 指定 `--first-at` 時，時間必須是未來的 RFC 3339 時間。
- 後續執行會按照 UTC 經過時間推進，不會隨 DST 改變。
- 停機期間錯過的週期不會補跑。

## 3. 查看、手動執行與 logs

### 3.1 查看 Flow

```bash
stoker flow list
stoker flow list --user <USER>
stoker flow show <FLOW_ID>
stoker flow history <FLOW_ID>
stoker flow occurrences <FLOW_ID>
```

- `flow list`：列出 Flow。
- `flow show`：查看 Flow 的設定與 task。
- `flow history`：查看 Flow 的執行歷史。
- `flow occurrences`：查看排程產生的 occurrence。

`stoker flow show <FLOW_ID>` 的每個 task 都會顯示 `cwd`。`cwd` 是該 task 執行 command 時使用的工作目錄。

### 3.2 手動執行 Flow

```bash
# 立即建立一次 manual run
stoker flow run <FLOW_ID>

# 指定 request ID，方便安全重送同一個請求
stoker flow run <FLOW_ID> --request-id <UUID>

# 在 manual run 開始後，取代一個明確的未來 occurrence
stoker flow run <FLOW_ID> --replace-next --request-id <UUID>
```

同一個 `--request-id` 可以安全重送。`--replace-next` 只會在 manual run 開始後取代一個未來 occurrence。

### 3.3 查看 run 與 logs

```bash
stoker flow show <FLOW_ID> --run <RUN_ID>
stoker flow show <FLOW_ID> --run <RUN_ID> --task <TASK_ID>

# --attempt <N> 從 1 起算。省略時顯示該 task 的全部 attempts。
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --follow
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N> --follow
```

### 3.4 取消 run 或 task

```bash
stoker flow cancel <FLOW_ID> --run <RUN_ID>
stoker flow cancel <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
```

### 3.5 完整操作範例

以下指令示範如何查看 `nightly` Flow、手動執行一次，以及查看和取消其中一個 task：

```bash
stoker flow show nightly
stoker flow history nightly
stoker flow occurrences nightly

stoker flow run nightly --request-id <UUID>
stoker flow run nightly --replace-next --request-id <UUID>

stoker flow show nightly --run <RUN_ID> --task refresh
stoker flow logs nightly --run <RUN_ID> --task refresh --attempt 1 --follow
stoker flow cancel nightly --run <RUN_ID>
stoker flow cancel nightly --run <RUN_ID> --task refresh
```

## 4. 編輯已提交的 Flow

已 commit 的 Flow 不能直接修改。請依序完成以下步驟：

1. `freeze`：凍結 Flow，建立 future draft。
2. 修改 task 或 schedule。
3. `apply`：套用 draft，並解除 freeze。

```bash
stoker flow edit begin nightly
stoker flow task update nightly publish --retries 2 --revision 0
stoker flow schedule set nightly --daily 01:00 --schedule-timezone Asia/Tokyo --revision 1
stoker flow edit apply nightly --revision 2
```

`--revision N` 是 draft 的版本號。stoker 會用它確認你修改的是最新 draft。版本不符時，修改不會套用。

以下是常用的 task 修改指令：

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
```

以下是 schedule 修改指令：

```text
stoker flow schedule set <FLOW_ID> --once-at <RFC3339>
stoker flow schedule set <FLOW_ID> --daily <HH:mm>
stoker flow schedule set <FLOW_ID> --daily <HH:mm> --schedule-timezone <IANA_ZONE>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --schedule-timezone <IANA_ZONE>
```

`once`、`daily` 和 `every` 是三種不同的 schedule 類型。它們不能互換，只能修改同類型的 schedule。

所有修改指令最後都可以加入 `--revision <N>`，用來確認 draft revision。

### 4.1 丟棄 draft、停用與啟用 Flow

```bash
stoker flow edit discard <FLOW_ID> --revision <N>
stoker flow disable nightly
stoker flow enable nightly
stoker request show <REQUEST_ID>
```

`flow edit discard` 只會丟棄 draft，Flow 仍然保持 frozen。執行 `flow edit apply` 才會套用 draft 並解除 freeze。

`task remove` 預設只作用於 `future`。如果要使用 `current` 或 `both`，必須提供 `--run`，而且 Flow 必須已經 freeze。

`disable` 只會停止未來的 automatic trigger，不會阻止合法的 manual run。

## 5. Standalone scheduled Job

在 `scheduled` mode 中，`stoker create` 也可以建立只執行一個 command 的 scheduled Job。建立後仍然要執行 `commit` 才會啟用。

```bash
stoker create --user alice --name frequent --cmd "python refresh.py" --every 15m --first-at 2099-01-01T10:00:00+09:00
stoker commit <JOB_ID>

stoker runs <JOB_ID>
stoker occurrences <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
stoker logs <JOB_ID> --run <RUN_ID> --follow
stoker cancel <JOB_ID> --run <RUN_ID>
```

建立 standalone scheduled Job 時，可以使用以下三種格式：

```text
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --once-at <RFC3339> [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --every <Nm|Nh> [--first-at <RFC3339>] [--retry <N>]
```

`--retry` 是 standalone Job 的重試次數。Flow task 使用的是 `--retries`。

如果要修改已 commit 的 schedule，請先 freeze，修改後再解除 freeze：

```bash
stoker freeze <JOB_ID>
stoker schedule set <JOB_ID> --every 2h --expected-draft-revision <N>
stoker unfreeze <JOB_ID> --expected-draft-revision <N>
```

其他常用操作如下：

```bash
stoker draft discard <JOB_ID> --expected-draft-revision <N>
stoker disable <JOB_ID>
stoker enable <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
```

`stoker draft discard` 只會丟棄 draft，Job 仍然保持 frozen。
`disable` 和 `enable` 控制 automatic trigger。
`run --skip-next` 會在 manual run 開始後取代一個未來 occurrence。

## 6. Declarative JSON source mode

如果需要把整組 Flow 定義放進版本控制或 code review，可以使用 JSON source mode。
如果你只是要建立一般 Flow，可以先跳過這一章。

source mode 決定 Flow 是由 CLI 逐筆修改，還是由 JSON 一次同步：

- `manual`：使用 `flow create`、`flow task` 和其他 Flow 指令逐筆修改。
- `sync`：以 JSON 檔案作為完整的 desired state，一次同步所有 Flow。desired state 就是你希望 workspace 最後變成的完整狀態。

既有 workspace 預設使用 `manual`。

### 6.1 同步前的安全流程

在開始前，請先確認：

- 沒有工作正在啟動、執行、取消或 recovery。
- 未完成的 Flow 已經 commit。
- frozen draft 已經 apply 或 discard。

接著依序執行：

```bash
# 1. 從目前 workspace 匯出 JSON。
# 匯出的檔案會包含 base revision 和 hash。
stoker flow export --dir ./flow-definitions

# 2. 編輯 JSON 後，鎖定 queue 並切換成 sync mode。
stoker queue lock
stoker flow source-mode sync

# 3. 先預覽結果，不會寫入任何狀態。
stoker flow sync <EXPORTED_JSON> --dry-run

# 4. 確認 dry-run 結果後，套用 Flow 定義。
stoker flow sync <EXPORTED_JSON>

# 5. 查看結果。sync 不會自動解除 queue lock。
stoker flow list
stoker queue unlock
```

切換 source mode 和正式 sync 都要求 queue 已經 locked。執行期間也不能有其他 execution。

`sync` mode 不允許以下修改指令：

- `flow create`
- `flow commit`
- `flow task`
- `flow schedule`
- `flow edit`
- `flow enable`
- `flow disable`

以下操作仍然可以使用：

- 查詢
- `run`
- `cancel`
- `log`
- `export`
- `snapshot`

### 6.2 回到 manual mode

如果要恢復逐筆修改，請保持 queue locked，先切回 `manual`，再解除 queue lock：

```bash
stoker flow source-mode manual
stoker queue unlock
```

### 6.3 JSON v1 格式

目前唯一支援的格式是 UTF-8 JSON。最安全的起點永遠是先執行 `flow export`。

```json
{
  "schema_version": 1,
  "base": {
    "revision": 12,
    "hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "flows": [
    {
      "id": "nightly",
      "name": "Nightly publish",
      "owner": "alice",
      "enabled": true,
      "schedule": {
        "type": "daily",
        "time": "23:30",
        "timezone": "Asia/Tokyo"
      },
      "tasks": [
        {
          "id": "publish",
          "name": "Publish",
          "cwd": {
            "default": ".",
            "windows": "D:/work/site",
            "linux": "/srv/site",
            "macos": "/Users/alice/site"
          },
          "command": "python publish.py",
          "retry": 1,
          "depend_mode": "all",
          "depends_on": []
        }
      ]
    }
  ]
}
```

#### Flow 與 task 的順序

- `flows` 陣列的順序就是 Flow 的排程順序。
- `tasks` 陣列的順序就是 task sequence。

#### schedule 欄位

`schedule.type` 可以是以下三種：

- `once`：使用 `at`。
- `daily`：使用 `time` 和 `timezone`。
- `periodic`：使用 `every`，也可以使用 `first_at`。

#### task 欄位

- `command`：要執行的 command。
- `retry`：task 失敗後的重試次數。
- `depend_mode`：多個 dependency 的判斷方式。
- `depends_on`：上游 task 的條件。每個條件格式如下：

```json
{
  "task_id": "...",
  "status": "succeeded|failed"
}
```

#### `cwd` 欄位

`cwd` 可以直接寫字串，也可以使用 platform map：

```json
{
  "default": ".",
  "windows": "D:/work/site",
  "linux": "/srv/site",
  "macos": "/Users/alice/site"
}
```

stoker 會依以下規則選擇工作目錄：

1. 先使用目前 OS 對應的值。
2. 沒有對應值時，使用 `default`。
3. 兩者都沒有時，使用 definition file 所在的目錄。

相對路徑也是以 definition file 所在的目錄解析。

- 實際選中的目錄必須已經存在。
- 其他 OS 的 override 會保留在檔案中，但不會在本機檢查是否存在。

#### JSON 驗證

Parser 會拒絕以下問題：

- 未知或重複欄位
- 缺少必要欄位
- 不支援的 schema
- 錯誤的 schedule
- 重複 ID
- 遺失 dependency
- 互相矛盾的 dependency edge
- 相依關係形成循環

拼字錯誤不會被忽略。

### 6.4 Revision、衝突與 no-op

`base.revision` 和 `base.hash` 代表你開始編輯時的 workspace 狀態。

如果其他人在你 export 後修改了 workspace，sync 會回報 stale conflict。處理方式如下：

1. 重新 export。
2. 把你的修改重新套用到新檔案。
3. 重新執行 dry-run。

如果兩個使用者從同一個 base 同時同步不同內容，只會有一個同步成功。

如果 desired state 和目前狀態完全相同，即使 base 已經過期，也會安全回報 no-op。這種情況不會增加 revision，也不會重複建立 snapshot。

### 6.5 Snapshot 與 source archive

```bash
stoker flow snapshot
```

`snapshot` 在 `manual` 和 `sync` 兩種 source mode 都可以使用。

- snapshot 會保存在 `<STOKER_HOME>/flows/snapshots/`。
- 檔名包含 UTC 時間、revision 和 hash。
- 正式 sync 覆蓋狀態前，會先建立 snapshot。
- sync 輸入會保存到 `<STOKER_HOME>/flows/sources/`。
- 等價內容會使用 SHA-256 去重。
- 完成的 artifact 會設為唯讀。
- 讀取 artifact 時會驗證 hash。
- 如果 artifact 無法寫入，這次 desired state 不會套用。

sync 使用的是完整 desired state。
JSON 中沒有列出的 Flow 會從未來排程移除，但以下內容不會因此被改寫：

- 既有 run
- attempt
- log
- 執行時保存的 definition snapshot

新增或變更 schedule 時，舊的 pending 或 reserved occurrence 會被新的 schedule 取代。

## 7. Recovery

如果重啟後 run 停在 `RECOVERING`，請先確認相關程序已經停止，再執行 reconcile：

```bash
stoker recovery reconcile <RUN_ID> --confirm-stopped
stoker queue unlock
```

所有 recovery 都完成後，才能解除 queue lock。

## 8. 指令命名與 help

所有 Flow 指令都以 `stoker flow` 開頭。

頂層 scheduled-job 指令只接受 standalone Job UUID。Flow ID 不能拿來代替 Job UUID。

需要完整 option 說明時，可以使用：

```bash
stoker flow --help
stoker flow task --help
```

也可以對各個子指令加上 `--help`。
