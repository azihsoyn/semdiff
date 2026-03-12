# semdiff

Structure-aware semantic diff tool.
Parses code at the AST level and performs **cross-file move detection**, **change classification**, and **repository-wide impact analysis**.

Designed to help review LLM-generated code and large-scale refactoring that are hard to follow with traditional line-based diffs.

## Features

- **AST-based structural diff** — Parses source using tree-sitter. Compares at the symbol level (functions, structs, enums, etc.) rather than lines
- **Automatic change classification** — Added / Deleted / Renamed / Moved / Extracted / Inlined / SignatureChanged / BodyChanged / VisibilityChanged
- **Cross-file move detection** — 5-phase algorithm: body hash exact match → name+body similarity → body similarity → extract detection → inline detection
- **Repo-aware impact analysis** — Call graph construction + similar code detection + pattern warnings. Visualizes the blast radius of changes
- **Git native** — Works as a git diff by default. No arguments = diff against the last commit
- **TUI** — Interactive viewer built with ratatui. Summary list, side-by-side diff, and impact analysis in a single screen
- **LLM review support** — The diff engine is LLM-independent. Structured change data can be fed to an LLM for review assistance

## Build

```bash
# Requirements: Rust toolchain (1.70+)
# Install from https://rustup.rs/

git clone https://github.com/azihsoyn/semdiff.git
cd semdiff
cargo build --release

# Binary is generated at target/release/semdiff
```

## Usage

By default, semdiff operates as a **git diff**. Arguments follow git range syntax.

### Basic (Git mode)

```bash
# No arguments: diff against HEAD (changes since last commit)
semdiff

# Last N commits
semdiff HEAD~3

# Between branches
semdiff main..feature-branch

# Between specific commits
semdiff abc123..def456

# Text output
semdiff HEAD -o text

# JSON output
semdiff main..feature -o json
```

### Repo-aware impact analysis

```bash
# With impact analysis (recommended)
semdiff main..feature --repo-analysis

# Specify impact depth (default: 2)
semdiff HEAD --repo-analysis --impact-depth 3
```

Impact analysis scans the entire repository and detects:

- **Affected Callers** — Call sites of changed functions (transitive callers are also tracked)
- **Similar Code** — Similar code in the repository (potential missed updates)
- **Pattern Warnings** — Warnings when functions with similar naming patterns are only partially updated

### Index (pre-compilation)

To speed up `--repo-analysis` on large repositories, you can pre-build the symbol DB, call graph, and similarity index.

```bash
# Build index at HEAD (saved to .semdiff/ directory)
semdiff index

# Build index at a specific ref
semdiff index --ref develop

# The index is automatically used during --repo-analysis
semdiff HEAD~3 --repo-analysis    # ~2s with cache hit
```

The index contains symbol information, call references, and MinHash signatures. It is automatically loaded during `--repo-analysis`. If the commit hash doesn't match, it falls back to a full scan.

### Directory / file comparison

To compare directories or files outside a git repository:

```bash
semdiff --dirs old_dir/ new_dir/
semdiff --dirs old.rs new.rs -o text
semdiff --dirs old/ new/ --repo-analysis
```

### LLM review

```bash
# Anthropic API
semdiff HEAD --llm-review --api-key $ANTHROPIC_API_KEY

# OpenAI API
semdiff HEAD --llm-review --llm-provider openai --api-key $OPENAI_API_KEY

# Can also be set via environment variable
export SEMDIFF_API_KEY=sk-...
semdiff HEAD --llm-review
```

The LLM receives structured change data (ChangeKind, symbol info, body diff) extracted by the algorithm — not the raw diff, but compact, focused input.

## TUI controls

```
Key          Action
─────────────────────────────
q            Quit
Tab          Cycle focus (Summary → Detail → Impact/Review)
j / k        Navigate / scroll
h / l        Horizontal scroll (Detail panel)
PgUp / PgDn  Scroll 10 lines
Home / End   Jump to first / last
v            Toggle bottom panel
b            Switch bottom panel (Impact ↔ Review)
```

```
┌──────────────────┬──────────────────────────────┐
│ Summary          │ Detail (side-by-side)        │
│ [MOV] process()  │ Old: main.rs      New: core.rs│
│ [SIG] transform()│                              │
│ [MOD] validate() │ -fn process(x: i32) {        │
│ [ADD] new_func() │ +fn process(x: i32, y: i32) {│
│ [DEL] old_func() │                              │
├──────────────────┴──────────────────────────────┤
│ Impact                                          │
│ Affected Callers (3)                             │
│  [HIGH] handler @ api.rs:42                      │
│ Similar Code (1)                                 │
│  [SIMILAR] process_v2 @ legacy.rs:10 (78%)       │
└─────────────────────────────────────────────────┘
```

## Supported languages

| Language    | Functions | Structs/Types | Methods | Constants | Call graph |
|-------------|-----------|---------------|---------|-----------|------------|
| Rust        | yes       | yes           | yes     | yes       | yes        |
| Go          | yes       | yes           | yes     | yes       | yes        |
| TypeScript  | yes       | yes           | yes     | yes       | yes        |
| TSX         | yes       | yes           | yes     | yes       | yes        |
| JavaScript  | yes       | yes           | yes     | yes       | yes        |
| Python      | yes       | yes           | -       | -         | yes        |
| Svelte      | yes       | yes           | yes     | yes       | yes        |

Svelte files: the `<script>` block is automatically extracted and parsed as TypeScript/TSX.

## Design principles

1. **Diff engine is LLM-independent** — AST parsing, similarity detection, move detection, and classification are all algorithmic
2. **LLM is for review assistance only** — Uses structured change units extracted by the algorithm as input
3. **Structure-based comparison** — Aims for human-readable diffs rather than minimum edit distance

## Cross-file move detection algorithm

1. **Exact body hash match** — O(1) detection using blake3 hash. Confidence: 95%
2. **Name + body similarity** — Compares body similarity of same-named symbols. Confidence: similarity × 0.9
3. **Body similarity only** — Candidates with ≥70% body similarity even if names differ. Confidence: similarity × 0.85
4. **Extract detection** — Checks if a new symbol's body is a substring of an old symbol
5. **Inline detection** — Checks if a deleted symbol's body is contained within a new symbol

## Repo-aware impact analysis

- **Call graph**: Traverses `call_expression` nodes via tree-sitter. Extracts call relationships from all source files and builds forward/reverse indexes
- **Similar code detection**: Fast repository-wide scan using Jaccard similarity of 4-gram shingles. Approximate Jaccard via MinHash for O(k) fast filtering. FNV hashing for shingle computation scales to large repositories
- **Pre-built index**: `semdiff index` saves symbol DB, call graph, and MinHash signatures to `.semdiff/`. Uses `git cat-file --batch` for fast batch loading
- **Risk assessment**: Signature change + has callers → High, body change + has callers → Medium, similar code not updated → Warning

## License

MIT
