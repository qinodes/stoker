# Flow と scheduled mode 詳細ガイド

このガイドでは、`scheduled` mode で使える 2 種類の仕事について説明します。

- **Flow**：複数の task を 1 つのワークフローにまとめたものです。
- **standalone scheduled Job**：Flow を使わず、1 つの command をスケジュール実行する Job です。

初めて stoker を使う場合は、まず [README](../README.ja.md) を参照してください。
1 回だけ実行する Job や、queue の順番に従って 1 件ずつ実行する処理は、[serial mode ガイド](serial.ja.md) を参照してください。

最初の Flow だけを作りたい場合は、「scheduled mode に切り替える」「最初の Flow を作成して実行する」「Flow の確認、手動実行、log の確認」の順に読んでください。後半では編集、JSON source mode、recovery を説明します。

## まず知っておきたい用語

- **Flow**：順番または依存関係に従って実行する task のまとまりです。
- **task**：Flow の中で実行する 1 つの command です。
- **run**：Flow を実際に 1 回実行した結果です。
- **occurrence**：schedule によって作られる 1 回分の実行予定です。
- **draft**：まだ適用されていない変更内容です。
- **standalone scheduled Job**：Flow を使わず、1 つの command だけをスケジュール実行する Job です。

## 1. scheduled mode に切り替える

Flow は `scheduled` mode でだけ使えます。mode を切り替える前に queue を lock してください。

```bash
stoker mode show
stoker queue lock
stoker mode set scheduled
stoker queue unlock
```

各 command の役割は次のとおりです。

1. `stoker mode show`：現在の mode を確認します。
2. `stoker queue lock`：切り替え中に新しい仕事が入らないよう、queue を一時的に lock します。
3. `stoker mode set scheduled`：`scheduled` mode に切り替えます。
4. `stoker queue unlock`：切り替えが終わったら queue の lock を解除します。

execution の開始、実行、cancel、cleanup、recovery のいずれかが進行中のときは mode を切り替えられません。完了してから切り替えてください。

## 2. 最初の Flow を作成して実行する

Flow は次の 3 段階で作成します。

1. schedule を含む draft Flow を作成します。
2. Flow に task を追加します。
3. Flow を commit して、schedule に従って実行できるようにします。

### 2.1 Flow を作成する

`<FLOW_ID>` は Flow の識別子です。`--name` は人が見る表示名です。2 つの値は異なっていても構いません。

Flow を作成するときは、次の 3 種類から schedule を 1 つ選びます。

```bash
# 1 回だけ実行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --once-at <RFC3339>

# 毎日実行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --daily <HH:mm> --schedule-timezone <IANA_ZONE>

# 一定間隔で実行
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh>
stoker flow create <FLOW_ID> --user <USER> --name <FLOW_NAME> --every <Nm|Nh> --first-at <RFC3339>
```

### 2.2 Flow に task を追加する

`flow task add` を実行する前に、task を実行したいディレクトリへ移動してください。現在のディレクトリが task の working directory になります。

`<TASK_ID>` は task の識別子です。`--name` は人が見る表示名です。2 つの値は異なっていても構いません。

```bash
# 依存関係なし
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>"

# 1 つの上流 task の成功を要求
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <TASK_ID>

# 2 つの上流 task の成功を要求。失敗した場合は 1 回 retry
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after <UPSTREAM_TASK_ID_1> --after <UPSTREAM_TASK_ID_2> --match all --retries 1

# 上流 task の失敗を要求
stoker flow task add <FLOW_ID> <TASK_ID> --name <TASK_NAME> --cmd "<COMMAND>" --after-failure <TASK_ID>
```

依存関係を追加するときは、次の点に注意してください。

- `--after <TASK_ID>`：上流 task が成功している必要があります。
- `--after-failure <TASK_ID>`：上流 task が失敗している必要があります。
- 複数の条件が必要な場合は、`--after` または `--after-failure` を繰り返します。
- `--match all`：すべての条件に一致したときだけ実行します。デフォルトです。
- `--match any`：少なくとも 1 つの条件に一致すれば実行します。
- `--retries <N>`：task 失敗後の retry 回数を指定します。`0` は retry なしです。

### 2.3 Flow を commit する

task を追加したら、Flow を検証して有効にします。

```bash
stoker flow commit <FLOW_ID>
```

### 2.4 完全な例

次の例では、毎日 23:30 に実行する Flow を作成します。`nightly` は `<FLOW_ID>` と表示名の両方に使われています。

```bash
stoker flow create nightly --user alice --name nightly --daily 23:30 --schedule-timezone Asia/Tokyo

# task を実行するディレクトリで実行してください。
stoker flow task add nightly refresh --name refresh --cmd "python refresh_cache.py"
stoker flow task add nightly publish --name publish --cmd "python publish_summary.py" --after refresh

# 2 つの上流 task が両方とも成功するのを待ちます。失敗した場合は 1 回 retry します。
stoker flow task add nightly notify --name notify --cmd "python send_notification.py" --after refresh --after publish --match all --retries 1

stoker flow commit nightly
```

別の schedule を使う場合は、最初の command の schedule option を置き換えます。

```bash
stoker flow create once --user alice --name once --once-at 2099-01-01T10:00:00+09:00
stoker flow create frequent --user alice --name frequent --every 15m --first-at 2099-01-01T10:00:00+09:00
```

### 2.5 schedule のルール

#### one-time schedule

`--once-at` には、秒と明示的な UTC offset を含む RFC 3339 timestamp を指定します。例：

```text
2099-01-01T23:30:00+09:00
```

#### daily schedule

`--daily` は `HH:mm` 形式です。timezone を指定する場合は `--schedule-timezone <IANA_ZONE>` を追加します。

- missed daily occurrence は後から実行されません。
- 夏時間（DST）によって存在しない時刻は skip されます。
- DST によって 2 回存在する時刻では、早い方の instant が使われます。

#### periodic schedule

`--every` に指定できるのは、小文字の整数と分または時間を組み合わせた値だけです。例は `1m`、`15m`、`1h`、`2h` です。

- 分の最小値は `1m` です。
- 時間の最小値は `1h` です。
- `--first-at` を省略すると、最初の実行は commit または schedule 更新後の 1 周期が経過したときです。
- `--first-at` を指定する場合は、未来の RFC 3339 timestamp でなければなりません。
- 以降の実行は UTC の経過時間で進み、DST によってずれません。
- scheduler の停止中に missed となった周期は後から実行されません。

## 3. Flow の確認、手動実行、log の確認

### 3.1 Flow を確認する

```bash
stoker flow list
stoker flow list --user <USER>
stoker flow show <FLOW_ID>
stoker flow history <FLOW_ID>
stoker flow occurrences <FLOW_ID>
```

- `flow list`：Flow を一覧表示します。
- `flow show`：Flow の設定と task を表示します。
- `flow history`：Flow の実行履歴を表示します。
- `flow occurrences`：schedule が作成した occurrence を表示します。

`stoker flow show <FLOW_ID>` に表示される各 task には `cwd` も含まれます。これは task が command を実行するときの working directory です。

### 3.2 Flow を手動実行する

```bash
# manual run を 1 回すぐに作成
stoker flow run <FLOW_ID>

# 同じ request を安全に retry できるよう request ID を指定
stoker flow run <FLOW_ID> --request-id <UUID>

# manual run の開始後、明確に指定された未来の occurrence を 1 件置き換え
stoker flow run <FLOW_ID> --replace-next --request-id <UUID>
```

同じ `--request-id` を使えば、request を安全に再送できます。`--replace-next` は manual run の開始後に、未来の occurrence を 1 件だけ置き換えます。

### 3.3 run と log を確認する

```bash
stoker flow show <FLOW_ID> --run <RUN_ID>
stoker flow show <FLOW_ID> --run <RUN_ID> --task <TASK_ID>

# --attempt <N> は 1 から始まります。省略すると task の全 attempt を表示します。
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N>
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --follow
stoker flow logs <FLOW_ID> --run <RUN_ID> --task <TASK_ID> --attempt <N> --follow
```

### 3.4 run または task を cancel する

```bash
stoker flow cancel <FLOW_ID> --run <RUN_ID>
stoker flow cancel <FLOW_ID> --run <RUN_ID> --task <TASK_ID>
```

### 3.5 完全な操作例

次の command では、`nightly` Flow の確認、1 回の手動実行、task の確認と cancel を行います。

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

## 4. commit 済みの Flow を編集する

commit 済みの Flow は直接編集できません。次の順番で操作します。

1. `freeze`：Flow を freeze して future draft を作成します。
2. task または schedule を編集します。
3. `apply`：draft を適用して Flow の freeze を解除します。

```bash
stoker flow edit begin nightly
stoker flow task update nightly publish --retries 2 --revision 0
stoker flow schedule set nightly --daily 01:00 --schedule-timezone Asia/Tokyo --revision 1
stoker flow edit apply nightly --revision 2
```

`--revision N` は draft のバージョンです。stoker は、最新の draft を編集していることを確認するために使います。バージョンが一致しない場合、変更は適用されません。

task を編集する主な command は次のとおりです。

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

schedule を編集する command は次のとおりです。

```text
stoker flow schedule set <FLOW_ID> --once-at <RFC3339>
stoker flow schedule set <FLOW_ID> --daily <HH:mm>
stoker flow schedule set <FLOW_ID> --daily <HH:mm> --schedule-timezone <IANA_ZONE>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh>
stoker flow schedule set <FLOW_ID> --every <Nm|Nh> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --first-at <RFC3339>
stoker flow schedule set <FLOW_ID> --schedule-timezone <IANA_ZONE>
```

`once`、`daily`、`every` は 3 種類の異なる schedule type です。相互に入れ替えることはできません。同じ type の option だけで編集してください。

すべての編集 command の最後に `--revision <N>` を追加して、draft revision を確認できます。

### 4.1 draft の破棄、Flow の disable と enable

```bash
stoker flow edit discard <FLOW_ID> --revision <N>
stoker flow disable nightly
stoker flow enable nightly
stoker request show <REQUEST_ID>
```

`flow edit discard` は draft だけを破棄します。Flow は frozen のままです。`flow edit apply` を実行すると draft が適用され、freeze が解除されます。

`task remove` はデフォルトで `future` に適用されます。`current` または `both` を使う場合は `--run` を指定し、Flow が freeze されていることを確認してください。

`disable` は未来の自動 trigger を停止します。正しい manual run は実行できます。

## 5. Standalone scheduled Job

`scheduled` mode では、`stoker create` で 1 つの command だけを実行する scheduled Job も作成できます。作成後に `commit` を実行して有効にする必要があります。

```bash
stoker create --user alice --name frequent --cmd "python refresh.py" --every 15m --first-at 2099-01-01T10:00:00+09:00
stoker commit <JOB_ID>

stoker runs <JOB_ID>
stoker occurrences <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
stoker logs <JOB_ID> --run <RUN_ID> --follow
stoker cancel <JOB_ID> --run <RUN_ID>
```

standalone scheduled Job は次の 3 つの形式で作成できます。

```text
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --once-at <RFC3339> [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --daily <HH:mm> [--schedule-timezone <IANA_ZONE>] [--retry <N>]
stoker create --user <USER> --name <NAME> --cmd <COMMAND> --every <Nm|Nh> [--first-at <RFC3339>] [--retry <N>]
```

`--retry` は standalone Job の retry 回数です。Flow の task では `--retries` を使います。

commit 済みの schedule を編集する場合は、先に freeze し、変更後に freeze を解除します。

```bash
stoker freeze <JOB_ID>
stoker schedule set <JOB_ID> --every 2h --expected-draft-revision <N>
stoker unfreeze <JOB_ID> --expected-draft-revision <N>
```

その他の主な操作は次のとおりです。

```bash
stoker draft discard <JOB_ID> --expected-draft-revision <N>
stoker disable <JOB_ID>
stoker enable <JOB_ID>
stoker run <JOB_ID> --skip-next --request-id <UUID>
```

`stoker draft discard` は draft だけを破棄します。Job は frozen のままです。
`disable` と `enable` は自動 trigger を制御します。
`run --skip-next` は manual run の開始後に未来の occurrence を 1 件置き換えます。

## 6. Declarative JSON source mode

Flow の定義一式を version control に保存したい場合や、code review の対象にしたい場合は JSON source mode を使います。
通常の Flow を作成するだけなら、この章は読み飛ばして構いません。

source mode は、Flow を CLI で 1 件ずつ編集するか、1 つの JSON ファイルからまとめて同期するかを決めます。

- `manual`：`flow create`、`flow task`、その他の Flow command で 1 件ずつ編集します。
- `sync`：JSON ファイルを完全な desired state として、すべての Flow を一度に同期します。desired state は workspace を最終的にどの状態にしたいかを表す全体の定義です。

既存の workspace はデフォルトで `manual` です。

### 6.1 安全な sync 手順

開始前に、次のことを確認してください。

- 起動中、実行中、cancel 中、recovery 中の仕事がない。
- 未完了の Flow が commit 済みである。
- frozen draft が apply または discard 済みである。

次の command を順番に実行します。

```bash
# 1. 現在の workspace から JSON を export。
# export したファイルには base revision と hash が含まれます。
stoker flow export --dir ./flow-definitions

# 2. JSON を編集してから queue を lock し、sync mode に切り替えます。
stoker queue lock
stoker flow source-mode sync

# 3. 状態を書き込まずに結果を確認します。
stoker flow sync <EXPORTED_JSON> --dry-run

# 4. dry-run の結果を確認したら、Flow の定義を適用します。
stoker flow sync <EXPORTED_JSON>

# 5. 結果を確認します。sync は queue を自動で unlock しません。
stoker flow list
stoker queue unlock
```

source mode の切り替えと正式な sync には、queue が locked であることが必要です。実行中に別の execution があってはいけません。

`sync` mode では、次の編集 command は使えません。

- `flow create`
- `flow commit`
- `flow task`
- `flow schedule`
- `flow edit`
- `flow enable`
- `flow disable`

次の操作は引き続き使えます。

- query
- `run`
- `cancel`
- `logs`
- `export`
- `snapshot`

### 6.2 manual mode に戻る

Flow を 1 件ずつ編集する状態に戻す場合は、queue を locked のまま `manual` に切り替え、その後 queue を unlock します。

```bash
stoker flow source-mode manual
stoker queue unlock
```

### 6.3 JSON v1 形式

現在サポートされている形式は UTF-8 JSON だけです。最も安全な開始方法は、先に `flow export` を実行することです。

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

#### Flow と task の順番

- `flows` 配列の順番が Flow の schedule 順です。
- `tasks` 配列の順番が task sequence です。

#### schedule の項目

`schedule.type` には次の 3 種類があります。

- `once`：`at` を使います。
- `daily`：`time` と `timezone` を使います。
- `periodic`：`every` を使い、`first_at` も指定できます。

#### task の項目

- `command`：実行する command です。
- `retry`：task 失敗後の retry 回数です。
- `depend_mode`：複数の dependency の判定方法です。
- `depends_on`：上流 task の条件です。各条件は次の形式です。

```json
{
  "task_id": "...",
  "status": "succeeded|failed"
}
```

#### `cwd` 項目

`cwd` には文字列を指定するか、platform map を使えます。

```json
{
  "default": ".",
  "windows": "D:/work/site",
  "linux": "/srv/site",
  "macos": "/Users/alice/site"
}
```

stoker は次の順番で working directory を選びます。

1. 現在の OS に対応する値。
2. 対応する値がなければ `default`。
3. どちらもなければ definition file があるディレクトリ。

相対パスも definition file があるディレクトリから解決されます。

- 実際に選ばれたディレクトリは、あらかじめ存在していなければなりません。
- 他の OS 用の override はファイルに残りますが、現在のマシンでは存在確認されません。

#### JSON の検証

Parser は次の問題を拒否します。

- 不明または重複した項目
- 必須項目の不足
- サポートされていない schema
- 不正な schedule
- 重複した ID
- dependency の不足
- 矛盾する dependency edge
- dependency の循環

スペルミスは無視されません。

### 6.4 Revision、conflict、no-op

`base.revision` と `base.hash` は、編集を開始したときの workspace の状態を表します。

export 後に誰かが workspace を変更すると、sync は stale conflict を返します。次の手順で解決してください。

1. もう一度 export します。
2. 新しいファイルに自分の変更をもう一度適用します。
3. dry-run をもう一度実行します。

同じ base から 2 人の user が異なる内容を同時に同期した場合、成功するのは 1 件だけです。

desired state が現在の状態と同じなら、base が古くても sync は安全に no-op を返します。この場合、revision は増えず、snapshot も重複して作られません。

### 6.5 Snapshot と source archive

```bash
stoker flow snapshot
```

`snapshot` は `manual` と `sync` のどちらの source mode でも使えます。

- snapshot は `<STOKER_HOME>/flows/snapshots/` に保存されます。
- ファイル名には UTC 時刻、revision、hash が含まれます。
- 正式な sync で状態を上書きする前に snapshot が作られます。
- sync の入力は `<STOKER_HOME>/flows/sources/` に保存されます。
- 同じ内容は SHA-256 で重複排除されます。
- 完了した artifact は read-only になります。
- artifact を読むときに hash が検証されます。
- artifact を書き込めない場合、desired state は適用されません。

sync は完全な desired state を使います。
JSON に存在しない Flow は未来の schedule から削除されます。ただし、次の内容は書き換えられません。

- 既存の run
- attempt
- log
- 実行中に保存された definition snapshot

schedule を追加または変更すると、古い pending または reserved の occurrence は新しい schedule に置き換えられます。

## 7. Recovery

再起動後も run が `RECOVERING` のままの場合は、まず関連するプロセスが停止していることを確認します。その後、reconcile を実行します。

```bash
stoker recovery reconcile <RUN_ID> --confirm-stopped
stoker queue unlock
```

すべての recovery が完了してから queue を unlock してください。

## 8. command 名と help

Flow の command はすべて `stoker flow` で始まります。

トップレベルの scheduled-job command が受け付けるのは standalone Job UUID だけです。Flow ID を Job UUID の代わりに使うことはできません。

option の詳細は次で確認できます。

```bash
stoker flow --help
stoker flow task --help
```

各 subcommand に `--help` を追加して確認することもできます。
