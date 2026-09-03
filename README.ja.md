# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows%20%7C%20macOS-blue)
![Version](https://img.shields.io/badge/version-0.5.4-informational)

[English](README.md) | [繁體中文](README.zh-TW.md) | 日本語

**stoker は軽量でクロスプラットフォームな、ローカルのコマンドライン（CLI）ジョブスケジューラーです。**
各 Job は作成時のディレクトリで実行されます。

## インストール

### Cargo をインストールしていない場合

[GitHub Releases](https://github.com/qinodes/stoker/releases) から環境に合うアーカイブをダウンロードし、`stoker` 実行ファイルを展開して、そのディレクトリを `PATH` に追加してください。

- Windows: `stoker-windows-x86_64.zip`
- Linux: `stoker-linux-x86_64.tar.gz`
- macOS Apple Silicon: `stoker-macos-arm64.tar.gz`

ダウンロードした実行ファイルは環境変数に自動で追加されません。実行ファイルのあるディレクトリを `PATH` に追加してください。

**Windows:** 「環境変数」の「ユーザー環境変数」で `Path` を編集し、実行ファイルのあるディレクトリを追加してから、ターミナルを再起動します。

**macOS／Linux:** `/path/to/stoker` を実行ファイルのある実際のディレクトリに置き換え、次の内容を `~/.zshrc`（macOS）または `~/.bashrc`（Linux）に追加してから、ターミナルを再起動します。

```bash
export PATH="/path/to/stoker:$PATH"
```

書き込んだ後、現在のターミナルにすぐ反映する場合は、次を実行します。

```bash
source ~/.bashrc  # Linux
source ~/.zshrc   # macOS
```

各 release には、プラットフォーム用の実行ファイルと `SHA256SUMS` も含まれます。

### Cargo がインストール済みの場合

```bash
cargo install stoker-engine
```

## クイックスタート

```bash

# scheduler をバックグラウンドで起動（Linux、macOS、Windows）
stoker start

# 対象ディレクトリで DRAFT Job を作成
stoker add --user <任意のユーザー名> --name <Job 名> --cmd <実行するコマンド>
# 例:
# stoker add --user alice --name exp-a --cmd python train.py --lr 0.0001

# 内容を確認して queue に追加（<JOB_ID> は直前の出力を使用）
# Job の詳細を表示
stoker show <JOB_ID>
# Job を送信（DRAFT -> QUEUED）
stoker commit <JOB_ID>

# 確認と管理

# scheduler の状態を確認
stoker status

# すべての Job の状態を表示
stoker jobs

# Job を絞り込み
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
# 絞り込み条件を組み合わせる
stoker jobs --user alice --state failed

# SUCCEEDED、FAILED、CANCELLED、LOST の Job とログを削除
# scheduler 実行中でも使用できます。
stoker clean

# 現在あるログを表示して終了
stoker logs <JOB_ID>

# 新しいログを Job の終了まで表示（Ctrl+C でも停止できます）
stoker logs -f <JOB_ID>

# Job をキャンセル（DRAFT、QUEUED、STARTING、RUNNING、CANCELLING）
stoker cancel <JOB_ID>

# scheduler を停止
# 実行中の Job はキャンセルされます。
# QUEUED の Job は次回の scheduler 起動時まで保持されます。
stoker stop

# 現在のバージョンを表示
stoker --version

# 最新版に更新
# 更新前に scheduler を停止してください。
stoker update

# アンインストール
# アンインストール前に scheduler を停止してください。
# Job データとログは Stoker のデータフォルダーに保持されます
# （macOS/Linux: ~/.stoker、Windows: %USERPROFILE%\.stoker）。
stoker uninstall
```

`--cmd` 以降はすべて実行プログラムへそのまま渡されます。stoker 自身のオプションは `--cmd` より前に指定してください。

`--user` は stoker の論理的な owner ラベルであり、OS アカウントや認証機能ではありません。

## Job の状態とキャンセル

| 状態 | 説明 |
| --- | --- |
| `DRAFT` | add 済みですが、まだ commit されていません。 |
| `QUEUED` | commit 済みで、実行待ちです。 |
| `STARTING` | scheduler が Job を取得し、ソースディレクトリとプロセスを準備しています。 |
| `RUNNING` | Job のプロセスが実行中です。 |
| `CANCELLING` | キャンセルが要求され、stoker がプロセスの停止とクリーンアップを行っています。 |
| `SUCCEEDED` | Job が正常に完了しました。 |
| `FAILED` | Job のプロセスが失敗したか、stoker が実行フローを完了できませんでした。 |
| `CANCELLED` | Job はキャンセルされました。 |
| `LOST` | scheduler の再起動時に、実行中だった Job の管理状態が失われました。 |

## 補足説明

ソースディレクトリのファイルに対して command が行った変更は保持されます。stoker はそのディレクトリ内のファイルを自動で変更または復元しません。

特定のバージョンをインストールする場合：

`cargo install stoker-engine --version <VERSION> --force`。

ログは `.stoker/runs/<JOB_ID>/stdout.log` と `.stoker/runs/<JOB_ID>/stderr.log` に保存されます。

## 対象範囲と制限

- 単一マシンの queue のみ対応。複数マシン、リモート実行、分散学習、GPU 割り当て、コンテナのスケジューリングには対応しません。
- add 時の作業ディレクトリが存在し、ディレクトリである必要があります。中のファイルは検査しません。
- Python/Conda/CUDA 環境、データセット、checkpoint、artifact、実験メトリクスは管理しません。
- stoker のアカウント、ログイン、権限管理はありません。`--user` は識別と絞り込み専用です。
