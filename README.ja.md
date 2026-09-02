# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-blue)]
[![Version](https://img.shields.io/badge/version-0.1.0-informational)](Cargo.toml)

[English](README.md) | [繁體中文](README.zh-TW.md) | 日本語

**stoker はローカル環境向けの Git 対応ジョブスケジューラーです。**
Git commit でコードのバージョンを固定し、コマンドとして実行できるタスクであればスケジュールできます。AI トレーニングは最初の利用例にすぎません。各ジョブは独立した Git worktree で実行されるため、現在の作業ディレクトリに影響しません。

## クイックスタート

Rust、Git、および Git repository が必要です。

```bash
# crates.io から CLI をインストール（Rust と Git が必要）
cargo install stoker-engine

# scheduler をバックグラウンドで起動（Linux / Windows）
stoker serve

# Git repository 内で DRAFT Job を作成
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# 内容を確認して queue に追加（<JOB_ID> は直前の出力を使用）
stoker show <JOB_ID>
stoker commit <JOB_ID>

# 確認と管理
stoker status
stoker jobs
stoker jobs --user alice
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>
stoker cancel <JOB_ID>
stoker stop
stoker --version
stoker update
stoker uninstall
```

`--cmd` 以降はすべて実行プログラムへそのまま渡されます。stoker 自身のオプションは `--cmd` より前に指定してください。

`--user` は論理的な owner ラベルであり、OS アカウントや認証機能ではありません。同じマシンを複数人で使う場合に、ジョブの識別と絞り込みに利用できます。

`jobs` は submit 済みの Job と `job_id`、owner、名前、状態を一覧表示します。

`stoker update` は更新可能なバージョンを表示し、明示的に `y` または `yes` を入力した場合のみ更新します。

特定バージョンを入れるには、`cargo install stoker-engine --version <VERSION> --force` を使用します。

`stoker uninstall` も明示的な確認が必要です。先に scheduler を停止してください。Job データと logs は Stoker のデータフォルダー（通常は `~/.stoker`）に保持されます。

## 対象範囲と制限

- 単一マシンの queue のみ対応。複数マシン、リモート実行、分散学習、GPU 割り当て、コンテナのスケジューリングには対応しません。
- Git repository とクリーンな working tree が必要です。Git がない場合や commit できない場合、Job は submit できません。
- Python/Conda/CUDA 環境、データセット、checkpoint、artifact、実験メトリクスは管理しません。
- stoker のアカウント、ログイン、権限管理はありません。`--user` は識別と絞り込み専用です。
