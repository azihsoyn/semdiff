# semdiff

構造理解ベースのセマンティック diff ツール。
AST レベルでコードを解析し、**ファイルを跨いだコード移動検出**・**変更分類**・**リポジトリ全体の影響分析**を行う。

通常の line diff では追いにくい LLM 生成コードや大規模リファクタリングのレビューを支援する。

## 特徴

- **AST ベースの構造 diff** — tree-sitter による構文解析。行単位ではなくシンボル（関数・構造体・enum 等）単位で比較
- **変更の自動分類** — Added / Deleted / Renamed / Moved / Extracted / Inlined / SignatureChanged / BodyChanged / VisibilityChanged
- **ファイル間移動検出** — body hash 完全一致 → 名前+body 類似度 → body 類似度 → 抽出/インライン検出の 5 フェーズアルゴリズム
- **Repo-aware 影響分析** — コールグラフ構築 + 類似コード検出 + パターン警告。変更の波及範囲を可視化
- **Git ネイティブ** — デフォルトで git diff として動作。引数なしで直前コミットとの diff
- **TUI** — ratatui ベースの対話的ビューア。サマリ一覧・詳細 diff・影響分析を一画面で閲覧
- **LLM レビュー支援** — diff エンジンは LLM 非依存。構造化された変更データを入力として LLM にレビュー補助を依頼可能

## ビルド

```bash
# 必要なもの: Rust toolchain (1.70+)
# https://rustup.rs/ でインストール

git clone <this-repo>
cd semdiff
cargo build --release

# バイナリは target/release/semdiff に生成される
```

## 使い方

基本は **git diff** として動作する。引数は git の range 指定。

### 基本（Git モード）

```bash
# 引数なし: HEAD との diff（直前コミットからの変更）
semdiff

# 直近 N コミット
semdiff HEAD~3

# ブランチ間の diff
semdiff main..feature-branch

# 特定コミット間
semdiff abc123..def456

# テキスト出力
semdiff HEAD -o text

# JSON 出力
semdiff main..feature -o json
```

### Repo-aware 影響分析

```bash
# 影響分析付き（推奨）
semdiff main..feature --repo-analysis

# 影響分析の深度を指定（デフォルト: 2）
semdiff HEAD --repo-analysis --impact-depth 3
```

影響分析ではリポジトリ全体を走査し、以下を検出する:

- **Affected Callers** — 変更された関数を呼び出している箇所（間接呼び出しも追跡）
- **Similar Code** — リポジトリ内の類似コード（更新漏れの可能性）
- **Pattern Warnings** — 名前パターンが似ている関数が片方だけ変更されている場合の警告

### インデックス（事前コンパイル）

巨大リポジトリでの `--repo-analysis` を高速化するため、シンボル DB・コールグラフ・類似度インデックスを事前に構築できる。

```bash
# HEAD でインデックス構築（.semdiff/ ディレクトリに保存）
semdiff index

# 特定 ref でインデックス構築
semdiff index --ref develop

# 以降は --repo-analysis 時にインデックスが自動的に使われる
semdiff HEAD~3 --repo-analysis    # キャッシュヒット時 ~2秒
```

インデックスにはシンボル情報・コールリファレンス・MinHash シグネチャが含まれ、`--repo-analysis` 実行時に自動的にロードされる。コミットハッシュが一致しない場合は自動的にフルスキャンにフォールバックする。

### ディレクトリ / ファイル比較（オプション）

git リポジトリ外や任意のディレクトリ同士を比較したい場合:

```bash
semdiff --dirs old_dir/ new_dir/
semdiff --dirs old.rs new.rs -o text
semdiff --dirs old/ new/ --repo-analysis
```

### LLM レビュー

```bash
# Anthropic API
semdiff HEAD --llm-review --api-key $ANTHROPIC_API_KEY

# OpenAI API
semdiff HEAD --llm-review --llm-provider openai --api-key $OPENAI_API_KEY

# 環境変数でも設定可能
export SEMDIFF_API_KEY=sk-...
semdiff HEAD --llm-review
```

LLM にはアルゴリズムで抽出した構造化変更データ（ChangeKind, シンボル情報, body diff）を送信する。raw diff 全体を投げるのではなく、コンパクトで焦点の絞られた入力を使用する。

## TUI 操作

```
キー         操作
─────────────────────────────
q            終了
Tab          パネル切替（Summary → Detail → Impact/Review）
j / k        上下移動 / スクロール
PgUp / PgDn  10行スクロール
Home / End   先頭 / 末尾
t            Detail タブ切替（Diff → Old Source → New Source）
v            下部パネル表示/非表示
b            下部パネル切替（Impact ↔ Review）
```

```
┌──────────────────┬──────────────────────────────┐
│ Summary          │ Detail                       │
│ [MOV] process()  │ Old: main.rs:1-10            │
│ [SIG] transform()│ New: core.rs:1-10            │
│ [MOD] validate() │                              │
│ [ADD] new_func() │ -fn process(x: i32) {        │
│ [DEL] old_func() │ +fn process(x: i32, y: i32) {│
├──────────────────┴──────────────────────────────┤
│ Impact                                          │
│ Affected Callers (3)                             │
│  [HIGH] handler @ api.rs:42                      │
│ Similar Code (1)                                 │
│  [SIMILAR] process_v2 @ legacy.rs:10 (78%)       │
└─────────────────────────────────────────────────┘
```

## 対応言語

| 言語         | 関数 | 構造体/型 | メソッド | 定数 | コールグラフ |
|-------------|------|-----------|----------|------|------------|
| Rust        | o    | o         | o        | o    | o          |
| Go          | o    | o         | o        | o    | o          |
| TypeScript  | o    | o         | o        | o    | o          |
| TSX         | o    | o         | o        | o    | o          |
| JavaScript  | o    | o         | o        | o    | o          |
| Python      | o    | o         | -        | -    | o          |
| Svelte      | o    | o         | o        | o    | o          |

Svelte は `<script>` ブロックを自動抽出して TypeScript/TSX として解析する。

## アーキテクチャ

```
src/
├── main.rs           CLI エントリポイント
├── cli.rs            clap 引数定義
├── git.rs            Git 統合（git コマンド経由）
├── ast/
│   ├── language.rs   言語検出・tree-sitter grammar
│   ├── parser.rs     ソースコード → AST
│   ├── query.rs      AST → Symbol 抽出（Rust/Go/TS/JS/Python）
│   ├── symbol.rs     Symbol 型・類似度計算・body 正規化
│   └── call_refs.rs  関数呼び出し参照の抽出
├── diff/
│   ├── mod.rs        diff オーケストレータ（Git / ディレクトリ両対応）
│   ├── change.rs     ChangeKind, SemanticChange, DiffResult
│   ├── matcher.rs    同一ファイル内シンボルマッチング
│   ├── classifier.rs マッチペアの変更種別分類
│   ├── cross_file.rs ファイル間移動検出（5 フェーズ）
│   └── body_diff.rs  関数本体の行レベル diff
├── repo/
│   ├── mod.rs        リポジトリ全体分析オーケストレータ
│   ├── call_graph.rs コールグラフ構築・クエリ
│   ├── similarity.rs shingle ベースの類似コード検出
│   └── impact.rs     影響分析（コールグラフ + 類似度 → リスク評価）
├── index.rs          事前コンパイル（シンボル DB / コールグラフ / MinHash）
├── tui/
│   ├── mod.rs        イベントループ・レイアウト
│   ├── app.rs        アプリ状態
│   ├── theme.rs      色・スタイル
│   └── panels/       Summary / Detail / Review / Impact パネル
├── llm/
│   ├── client.rs     Anthropic/OpenAI API クライアント
│   ├── prompt.rs     構造化変更データ → プロンプト生成
│   └── review.rs     ReviewResult 型
└── output/
    ├── text.rs       テキスト出力
    └── json.rs       JSON 出力
```

### 設計原則

1. **diff エンジンは LLM 非依存** — AST 解析、類似判定、move detection、classification は全てアルゴリズムで完結
2. **LLM はレビュー補助のみ** — アルゴリズムで抽出した構造化 change unit を入力として活用
3. **構造ベースの比較** — 最小編集距離ではなく、人間が理解しやすい diff を目指す

### ファイル間移動検出アルゴリズム

1. **Exact body hash match** — blake3 ハッシュで O(1) 完全一致検出。信頼度 95%
2. **Name + body similarity** — 同名シンボルの body 類似度比較。信頼度 = 類似度 × 0.9
3. **Body similarity only** — 名前が異なっても body が 70% 以上類似なら候補。信頼度 = 類似度 × 0.85
4. **Extract detection** — 新シンボルの body が旧シンボルの部分文字列かチェック
5. **Inline detection** — 削除シンボルの body が新シンボルに含まれるかチェック

### Repo-aware 影響分析

- **コールグラフ**: tree-sitter で `call_expression` ノードを走査。全ソースファイルから呼び出し関係を抽出し、順方向・逆方向インデックスを構築
- **類似コード検出**: 4-gram shingle の Jaccard 類似度で高速にリポジトリ全体を走査。MinHash による近似 Jaccard で O(k) 高速フィルタリング。FNV ハッシュによるシングル化で大規模リポジトリにも対応
- **事前インデックス**: `semdiff index` でシンボル DB・コールグラフ・MinHash シグネチャを `.semdiff/` に保存。`git cat-file --batch` によるバッチ読み込みで高速化
- **リスク評価**: シグネチャ変更 + 呼び出し元あり → High、body 変更 + 呼び出し元あり → Medium、類似コードの更新漏れ → Warning

## ライセンス

MIT
