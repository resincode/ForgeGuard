# ForgeGuard 9.8 Roadmap

## Menjadikan ForgeGuard OSS Guardrail Terbaik untuk Vibe Coding yang Memaksa Hasil AI Agent Mendekati Standar Senior Engineer

**Repository:** `https://github.com/suiflex/ForgeGuard`  
**Audit basis:** ForgeGuard `main`, release v0.14.0 (22 Agustus 2026)  
**Target:** technical/product score minimal **9.8/10** untuk kategori *AI Agent Engineering Control Plane / Deterministic Completion Gate*.

---

# 1. Executive Summary

ForgeGuard sudah berkembang jauh dari sekadar hook untuk mencegah AI coding agent berhenti terlalu cepat. Versi sekarang sudah memiliki dua lapisan utama:

1. **General Guard** — menjaga pekerjaan non-code seperti QA, product discovery, architecture, security review, business analysis, database work, content, statistik, dan profesi custom.
2. **Code Guard** — menjaga pekerjaan engineering di repository dengan workflow `inspect → design → implement → test → review → verify`, changed-code static analysis, command execution, evidence, baseline, coverage, SARIF, dan Stop-hook enforcement.

Secara teknis ForgeGuard saat ini layak dinilai sekitar **8.7/10 overall**, dengan kekuatan terbesar pada:

- agent completion enforcement,
- deterministic goal contract,
- hill-climbability,
- bounded auto-poke,
- token efficiency,
- multi-agent integration,
- changed-code philosophy,
- local deterministic verification tanpa LLM kedua.

Namun kalau targetnya adalah:

> **OSS terbaik untuk vibe coding agar hasil AI coding agent tidak bodoh, tidak asal selesai, tidak mengarang bahwa sesuatu sudah benar, dan dipaksa bekerja seperti engineer senior**, 

maka static analysis saja tidak cukup.

ForgeGuard harus berevolusi menjadi:

> **Engineering Control Plane untuk AI Agent — sistem yang mengatur objective, scope, architecture, implementation, testing, runtime evidence, security, performance, review, dan final completion secara deterministik.**

Target 9.8 bukan berarti ForgeGuard harus menggantikan SonarQube, Semgrep, compiler, profiler, database optimizer, test framework, atau CI. Justru sebaliknya: ForgeGuard harus menjadi **orchestrator dan enforcement layer** yang memastikan seluruh bukti tersebut benar-benar digunakan sebelum agent boleh mengklaim pekerjaan selesai.

---

# 2. Prinsip Utama: Jangan Mencoba Membuat Model Menjadi Senior — Paksa Prosesnya Menjadi Senior

Kesalahan positioning yang perlu dihindari:

> “ForgeGuard membuat AI menjadi senior engineer.”

Itu terlalu absolut.

Model tetap bisa salah. Model kecil tetap bisa lebih lemah dari model frontier. Model frontier pun tetap bisa salah memahami requirement, membuat abstraksi buruk, berhenti terlalu cepat, atau menjalankan test yang tidak membuktikan objective.

Positioning yang lebih kuat:

> **ForgeGuard forces AI coding agents to follow senior-engineering discipline before they are allowed to finish.**

Atau:

> **ForgeGuard turns vibe coding into verifiable engineering.**

Model boleh “vibe”. ForgeGuard tidak.

ForgeGuard harus memastikan hasil akhir memenuhi kontrak:

```text
User intent
   ↓
Measurable objective
   ↓
Declared scope
   ↓
Architecture constraints
   ↓
Implementation
   ↓
Static analysis
   ↓
Tests
   ↓
Runtime evidence
   ↓
Security/performance/data checks
   ↓
Diff review
   ↓
Acceptance coverage
   ↓
Independent completion decision
   ↓
DONE
```

---

# 3. Current ForgeGuard Audit

## 3.1 Current Overall Score

| Dimension | Current Score |
|---|---:|
| Problem selection | 9.7 |
| Product differentiation | 9.4 |
| Agent lifecycle architecture | 9.3 |
| Hill-climb / anti-poke | 9.3 |
| Token efficiency | 9.5 |
| Cross-agent support | 9.1 |
| Code quality engine | 8.1 |
| Security analyzer | 7.8 |
| Multi-language | 8.7 |
| General Guard | 8.9 |
| DX / distribution | 9.2 |
| Testing / CI discipline | 8.8 |
| Documentation consistency | 7.4 |
| Enterprise readiness | 6.7 |
| Ecosystem validation | 5.2 |
| **Overall technical product** | **8.7 / 10** |

Target akhir:

| Dimension | Target |
|---|---:|
| Agent lifecycle architecture | 10.0 |
| Goal/task correctness | 9.9 |
| Architecture verification | 9.8 |
| Code-quality verification | 9.7 |
| Runtime verification | 9.9 |
| Security/data safety | 9.7 |
| Testing confidence | 9.9 |
| Cross-agent support | 9.8 |
| Token efficiency | 9.8 |
| Evidence/provenance integrity | 9.8 |
| CI/PR delivery integrity | 9.8 |
| Real-world benchmark confidence | 9.8 |
| Documentation/release maturity | 9.8 |
| Ecosystem/adoption | 9.0+ |
| **Target overall** | **9.8 / 10** |

---

# 4. Fitur ForgeGuard yang Sudah Kuat

## 4.1 General Guard

General Guard memungkinkan ForgeGuard bekerja tanpa project initialization untuk pekerjaan seperti:

- Product Owner
- QA
- Security Engineer
- Business Analyst
- Database Administrator
- Architect
- Content Creator
- Statistician
- custom profession/profile

State yang bisa dijaga antara lain:

- objective,
- profile,
- metric,
- baseline,
- target,
- guardrails,
- verification method,
- TODO,
- acceptance criteria,
- evidence,
- evidence source/provenance,
- artifact,
- confidence,
- file scope,
- MCP/resource scope.

Ini membuat ForgeGuard bukan sekadar coding tool.

---

## 4.2 Code Guard

Code Guard menjalankan workflow:

```text
inspect → design → implement → test → review → verify
```

Agent tidak boleh menganggap `test passed` sama dengan `task completed`.

Stop hook menjadi titik enforcement sebelum completion.

---

## 4.3 Hill-Climbability

ForgeGuard sudah punya deterministic goal completeness score:

| Field | Score |
|---|---:|
| Metric | 20 |
| Baseline | 20 |
| Target | 20 |
| Guardrail | 20 |
| Verification | 20 |
| **Total** | **100** |

Contoh objective buruk:

```text
Improve API performance.
```

Contoh objective yang hill-climbable:

```text
Metric      : p95 /search latency
Baseline    : 900 ms
Target      : < 300 ms
Guardrail   : error rate tidak naik
Verification: benchmark + regression test
```

Fitur ini harus dipertahankan sebagai salah satu identitas ForgeGuard.

---

## 4.4 Auto-Poke dan Anti-Premature Completion

ForgeGuard sudah punya:

- retry budget,
- no-progress detection,
- state-change detection,
- bounded auto-poke,
- default max auto-pokes,
- hard limit,
- blocker state.

Ini lebih matang daripada sekadar “kalau belum selesai, bilang continue”.

---

## 4.5 Changed-Code Philosophy

ForgeGuard mendukung:

- changed-line review,
- base revision comparison,
- baseline existing findings,
- changed-function complexity,
- changed-line LCOV coverage.

Ini tepat untuk repository besar karena tidak memaksa agent membersihkan seluruh legacy debt.

Prinsipnya:

> **Clean as you code.**

---

## 4.6 Static Analysis

Current implementation memiliki sekitar 25 rule IDs yang mencakup:

### Algorithm

- nested iteration,
- repeated linear lookup,
- sorting in iteration.

### Database

- database operation in iteration,
- possible database operation in iteration,
- `SELECT *`.

### Networking

- external request inside iteration,
- possible external request inside iteration.

### Concurrency

- unbounded parallel fan-out.

### Complexity

- high function complexity.

### Security

- hardcoded credentials,
- dynamic sensitive operations,
- taint to sensitive sink,
- weak crypto/TLS,
- unsafe deserialization,
- XSS/raw HTML sink,
- tainted path to filesystem sink.

### Authorization

- mutating routes requiring access-control review.

### Error Handling

- swallowed exceptions.

### Coverage

- changed-line coverage policy.

### Duplication

- exact duplicate,
- renamed Type-2 clone,
- similar business operations.

### Architecture

- large inline data literal.

### Parser Reliability

- structural analysis skipped due to parse failure.

---

## 4.7 Language Support

Current capability registry covers language groups such as:

- JavaScript / TypeScript
- Python
- Rust
- Go
- Java / Kotlin
- C#
- C / C++
- Ruby
- PHP
- Swift
- Dart
- Shell
- Zig
- Lua
- Scala
- Solidity
- R
- Elixir / Erlang
- HCL / Terraform

Namun semantic provenance pack saat ini paling kuat pada:

- JavaScript / TypeScript
- Python
- Rust
- Go

---

## 4.8 Supply Chain

ForgeGuard sudah dapat mendeteksi atau mengintegrasikan command seperti:

- dependency audit,
- license inventory/policy,
- SBOM.

Dengan prinsip bagus:

- disabled by default untuk network-sensitive checks,
- dependency-change aware,
- cached berdasarkan fingerprint,
- failure selalu rerun.

---

## 4.9 Agent Support

Current target mencakup:

- Codex
- Claude Code
- Cursor
- OpenCode
- Hermes
- OpenClaw
- Antigravity
- Windsurf
- GitHub Copilot
- Cline
- Roo Code

Level enforcement berbeda tergantung lifecycle API host.

Native/hard completion integration tersedia pada host yang memberikan Stop/finalize lifecycle yang sesuai.

Host lain menggunakan policy enforcement.

Ini perlu terus dijelaskan secara jujur.

---

# 5. Masalah Utama yang Masih Menghalangi Score 9.8

ForgeGuard saat ini sudah kuat sebagai guardrail, tetapi untuk benar-benar menjadi *best-in-class vibe coding engineering layer*, ada gap besar berikut.

---

# 6. Priority P0 — Fitur yang Harus Ada untuk Naik dari 8.7 ke 9.3+

## 6.1 Real Runtime Verification

### Masalah

Static analysis dan test command belum selalu membuktikan bahwa software benar-benar bekerja.

Agent bisa menjalankan:

```text
npm test
```

lalu semuanya hijau, tetapi:

- aplikasi gagal boot,
- endpoint salah,
- database migration rusak,
- frontend blank,
- HTTP contract salah,
- integration flow gagal,
- service dependency tidak terhubung.

### Tambahkan: Runtime Evidence Gate

ForgeGuard perlu punya konsep:

```text
forgeguard verify runtime
```

atau configuration contract:

```toml
[[runtime_checks]]
name = "service-health"
start = "docker compose up -d"
probe = "curl -f http://localhost:3000/health"
timeout_seconds = 120
required = true
```

Runtime evidence harus bisa meliputi:

- process boots,
- health check passes,
- expected port listening,
- HTTP endpoint response,
- CLI command result,
- browser flow result,
- database connection,
- migration up/down verification,
- container health,
- integration dependency readiness.

### Impact

Ini salah satu langkah terbesar untuk mengubah:

> “tests pass”

menjadi:

> “system actually works”.

**Priority:** P0  
**Score impact:** sangat tinggi.

---

## 6.2 Test Adequacy, Bukan Hanya Test Execution

### Masalah

Agent bisa membuat test bodoh yang selalu pass.

Contoh:

```javascript
expect(true).toBe(true)
```

atau membuat test yang tidak menyentuh behavior baru.

### Tambahkan: Test Quality Gate

ForgeGuard harus mengecek:

1. apakah changed production code punya mapped tests,
2. apakah test benar-benar mengeksekusi changed behavior,
3. apakah assertion meaningful,
4. apakah negative case tersedia,
5. apakah error path diuji,
6. apakah test hanya snapshot/golden tanpa validation penting.

Target feature:

```text
FG-TEST-001 Changed code has no relevant test
FG-TEST-002 Test contains trivial assertion
FG-TEST-003 Test does not exercise changed symbol
FG-TEST-004 Error path introduced without negative test
FG-TEST-005 Changed branch lacks coverage evidence
```

Lanjutkan roadmap `changed-function test mapping` menjadi fitur inti.

### Advanced

Gunakan dynamic coverage mapping:

```text
changed function
   ↓
coverage trace
   ↓
which tests executed it?
```

Bukan sekadar coverage line percentage.

**Priority:** P0.

---

## 6.3 Mutation Testing Integration

### Masalah

100% coverage tidak berarti test suite bagus.

### Solusi

Optional mutation gate:

```text
changed code
  ↓
controlled mutations
  ↓
existing tests must kill them
```

Integrasi native tool per ecosystem, misalnya:

- Stryker
- mutmut
- cargo-mutants
- PIT

ForgeGuard tidak perlu membuat mutation engine sendiri.

Ia cukup:

- detect tool,
- run changed-scope mutation testing,
- enforce mutation score threshold,
- record evidence.

**Priority:** P0/P1.

---

## 6.4 Architecture Contract

### Masalah

Senior engineer bukan hanya membuat code yang pass.

Ia menjaga:

- dependency direction,
- module boundaries,
- layering,
- public API,
- data ownership,
- service contracts,
- architecture invariants.

Agent sering menghasilkan code benar secara lokal tetapi merusak arsitektur.

### Tambahkan: Architecture Guard

Contoh `.forgeguard/architecture.toml`:

```toml
[[layers]]
name = "domain"
paths = ["src/domain/**"]
may_depend_on = []

[[layers]]
name = "application"
paths = ["src/application/**"]
may_depend_on = ["domain"]

[[layers]]
name = "infrastructure"
paths = ["src/infrastructure/**"]
may_depend_on = ["application", "domain"]
```

Rule:

```text
FG-ARCH-010 forbidden dependency direction
FG-ARCH-011 module boundary violation
FG-ARCH-012 circular dependency introduced
FG-ARCH-013 public API changed without declared contract
FG-ARCH-014 cross-service ownership violation
```

Support architecture patterns:

- Clean Architecture
- Hexagonal
- DDD boundaries
- modular monolith
- microservices
- frontend feature slices
- mobile layers.

**Priority:** P0.

---

## 6.5 API / Schema Contract Verification

Agent sering merusak integrasi karena mengganti:

- request DTO,
- response DTO,
- REST endpoint,
- GraphQL schema,
- protobuf,
- OpenAPI,
- event contract,
- database schema.

### Tambahkan Contract Guard

Deteksi breaking change terhadap:

- OpenAPI
- GraphQL schema
- Protobuf
- AsyncAPI
- JSON Schema
- Avro
- database migrations

Contoh:

```text
FG-CONTRACT-001 breaking REST contract
FG-CONTRACT-002 required response field removed
FG-CONTRACT-003 protobuf incompatible change
FG-CONTRACT-004 event payload incompatible
FG-CONTRACT-005 migration destructive change
```

**Priority:** P0.

---

# 7. Priority P0 — Evidence dan Provenance Harus Naik Kelas

## 7.1 Execution Attestation

Sekarang user/agent bisa menyatakan:

```text
--source mcp:playwright
```

Namun string itu sendiri belum membuktikan tool benar-benar dipakai.

### Tambahkan Verified Evidence Receipt

ForgeGuard perlu menghasilkan receipt setelah command/tool execution:

```json
{
  "tool": "playwright",
  "session": "abc",
  "started_at": "...",
  "finished_at": "...",
  "input_hash": "...",
  "output_hash": "...",
  "exit_code": 0,
  "artifact_hash": "...",
  "worktree_fingerprint": "..."
}
```

Kemudian acceptance criteria hanya boleh menggunakan evidence receipt yang:

- berasal dari session tersebut,
- terkait current repository fingerprint,
- belum stale,
- tidak dimodifikasi.

### Optional signing

Local signing key atau ephemeral key:

```text
ForgeGuard Execution Receipt
     ↓
SHA-256
     ↓
optional signature
```

Ini membuat evidence jauh lebih kuat.

**Priority:** P0.

---

## 7.2 State-Bound Evidence

Evidence harus invalid jika code berubah setelah verification.

Contoh:

```text
Tests passed
commit state A

agent changes production code
state B

old evidence != valid anymore
```

Setiap evidence harus bound ke:

- Git tree hash / worktree fingerprint,
- task state hash,
- config hash,
- relevant dependency fingerprint.

Jika salah satu berubah, ForgeGuard rerun evidence yang terdampak.

Ini sangat penting untuk menghindari “test pass sebelum perubahan terakhir”.

**Priority:** P0.

---

# 8. Priority P0 — Senior-Level Planning Guard

## 8.1 Change Plan Contract

Sebelum edit besar, agent harus menyatakan:

- problem interpretation,
- affected modules,
- expected behavior,
- technical approach,
- risk,
- files expected to change,
- verification plan.

Contoh:

```text
forgeguard task plan
```

atau state:

```yaml
plan:
  problem: "..."
  affected_components:
    - auth-service
    - admin-ui
  expected_files:
    - src/auth/**
  risks:
    - token migration
    - backward compatibility
  verification:
    - unit
    - integration
    - smoke
```

ForgeGuard kemudian membandingkan actual diff dengan plan.

### Rule

```text
FG-PLAN-001 major changed component was not planned
FG-PLAN-002 declared implementation approach drifted
FG-PLAN-003 verification plan incomplete for risk tier
```

**Priority:** P0.

---

## 8.2 Automatic Risk Classification

Senior engineer memperlakukan perubahan berbeda berdasarkan risikonya.

Contoh high risk:

- auth,
- payments,
- permission/RBAC,
- migrations,
- production infra,
- dependency upgrade,
- CI pipeline,
- encryption,
- transaction logic,
- concurrency,
- irreversible data operation.

ForgeGuard harus otomatis menentukan risk tier:

```text
LOW
MEDIUM
HIGH
CRITICAL
```

Lalu verification minimum berbeda.

### Example

```text
LOW:
  lint + unit test

MEDIUM:
  lint + unit + integration

HIGH:
  architecture + integration + runtime + security

CRITICAL:
  human confirmation + full evidence + rollback proof
```

Ini salah satu fitur yang paling membuat workflow terasa “senior”.

**Priority:** P0.

---

# 9. Priority P0 — Database Safety Pack

Database adalah tempat AI coding agent sangat berbahaya.

## Fitur wajib

### Migration analysis

Deteksi:

- DROP COLUMN,
- DROP TABLE,
- rename destructive,
- type narrowing,
- NOT NULL tanpa migration strategy,
- index removal,
- table lock risk,
- large backfill,
- irreversible migration.

### Query-plan evidence

Untuk query baru/berubah:

```text
EXPLAIN
EXPLAIN ANALYZE
```

record evidence:

- sequential scan,
- estimated rows,
- actual rows,
- index usage,
- cost regression.

### N+1 runtime verification

Current static DB-in-loop detection bagus, tetapi runtime evidence akan jauh lebih kuat.

Target:

```text
baseline query count = 4
new query count      = 104
→ BLOCK
```

### Transaction safety

Deteksi flow seperti:

```text
read → write → write
```

tanpa atomic transaction pada domain kritikal.

**Priority:** P0.

---

# 10. Priority P1 — Compiler-Backed Semantics

ForgeGuard saat ini AST-assisted.

Untuk score 9.8, semantic pack perlu naik dari:

```text
Tree-sitter + bounded provenance
```

menjadi optional compiler-backed adapters.

### JavaScript / TypeScript

- TypeScript compiler API
- tsserver data

### Rust

- rustc metadata / cargo check JSON

### Go

- `go/types`

### Python

- pyright/mypy integration

### Java/Kotlin

- compiler/LSP symbol information

Tujuan:

- actual symbol resolution,
- type-aware data flow,
- overload resolution,
- cross-file call graph,
- return-value provenance,
- true interface impact.

ForgeGuard tidak perlu mem-build compiler sendiri.

Gunakan existing compiler ecosystem sebagai evidence provider.

**Priority:** P1.

---

# 11. Priority P1 — Advanced Security Guard

Current security scan sudah bagus sebagai awal.

Agar mencapai 9.8:

## Tambahkan

- SQL injection context,
- SSRF,
- command injection,
- template injection,
- path traversal strengthening,
- insecure random,
- missing authz on sensitive reads,
- IDOR hotspot,
- insecure CORS,
- JWT verification pitfalls,
- dangerous file upload,
- unsafe redirect,
- credential logging,
- sensitive data exposure.

Namun jangan menjadikan ForgeGuard pesaing Semgrep ruleset secara horizontal.

Fokus rule yang **AI-agent specific**:

- suspicious shortcut implementation,
- auth bypass added to “make tests pass”,
- validation removed,
- security control disabled,
- `verify=false`,
- `rejectUnauthorized=false`,
- `*` permission,
- CORS wildcard,
- temporary debug endpoints,
- default passwords,
- bypass flag.

Ini sangat cocok dengan masalah vibe coding.

---

# 12. Priority P1 — Performance Regression Guard

Tambahkan performance contract:

```text
Metric    : p95 latency
Baseline  : 220 ms
Threshold : <= 250 ms
```

Support:

- benchmark command,
- CPU benchmark,
- memory,
- allocation,
- bundle size,
- binary size,
- startup time,
- query latency,
- API latency.

Rule:

```text
FG-PERF-001 benchmark regression
FG-PERF-002 memory regression
FG-PERF-003 bundle-size regression
FG-PERF-004 query-plan regression
```

Senior engineer tidak hanya mengecek benar/salah, tetapi juga regresi kualitas.

---

# 13. Priority P1 — Frontend / Mobile Runtime Guard

Untuk vibe coding, UI adalah area di mana agent sering “menganggap benar” tanpa melihat hasilnya.

ForgeGuard harus mendukung browser/device evidence.

## Browser

- app loads,
- no console error,
- no unhandled promise rejection,
- expected network requests,
- screenshot comparison,
- accessibility violations,
- responsive breakpoints,
- interaction flow.

## Mobile

- app builds,
- emulator boots,
- target screen opens,
- crash logs absent,
- navigation flow works,
- screenshot artifact.

Tidak perlu membuat Playwright/Appium sendiri.

ForgeGuard menjadi enforcement layer atas existing tools.

---

# 14. Priority P1 — Observability-Aware Verification

Senior engineer melihat log, metric, dan trace.

ForgeGuard bisa punya optional runtime observation contract:

```yaml
observability:
  forbid_log_patterns:
    - "panic"
    - "UnhandledPromiseRejection"
    - "SQLSTATE"
  max_error_count: 0
```

Bisa menerima evidence dari:

- local logs,
- OpenTelemetry,
- application logs,
- browser console,
- test container logs.

---

# 15. Priority P1 — Rollback and Recovery Proof

Untuk high-risk change:

> “Bagaimana kalau perubahan gagal?”

Agent harus membuktikan rollback.

Contoh migration:

```text
migration up passes
migration down passes
migration up again passes
```

Contoh deployment config:

- previous config can be restored,
- feature flag exists,
- migration backward compatibility defined.

Target rule:

```text
FG-RISK-010 high-risk change has no rollback evidence
```

---

# 16. Priority P1 — Independent Final Review Phase

Agent yang menulis code cenderung bias terhadap hasil sendiri.

ForgeGuard harus menyediakan **deterministic independent review pass**, tetap tanpa LLM wajib.

Review phase harus membandingkan:

```text
Objective
Plan
Diff
Tests
Runtime evidence
Acceptance criteria
Architecture rules
Security findings
Performance findings
```

Dan membuat final checklist secara otomatis.

Optional mode boleh menggunakan secondary agent/LLM reviewer, tetapi:

- tidak wajib,
- tidak menjadi source of truth,
- verdict deterministik tetap authoritative.

Ini mempertahankan keunggulan no-extra-LLM-call ForgeGuard.

---

# 17. Priority P1 — Completion Confidence Harus Dihitung dari Evidence, Bukan Model

Saat ini model confidence bersifat advisory.

Tambahkan ForgeGuard **Evidence Confidence Score** sendiri.

Contoh:

```text
Goal contract completeness    100
Acceptance coverage           100
Required checks               100
Runtime verification          100
Changed test mapping           90
Architecture                    100
Security                         95
Performance                      100
-----------------------------------
ForgeGuard confidence            98
```

Ini bukan AI confidence.

Ini **evidence completeness score**.

Model bisa bilang confidence 100, tetapi ForgeGuard evidence score 62 → belum selesai.

---

# 18. Priority P1 — Quality Budget / Regression Budget

Tidak semua finding harus nol.

Buat konsep:

```text
quality budget
```

Contoh:

```toml
[quality_budget]
new_errors = 0
new_warnings = 0
complexity_delta = 0
coverage_delta = 0
security_delta = 0
```

Atau repo legacy:

```text
must not become worse than baseline
```

Ini lebih fleksibel dan senior-friendly daripada semua rule = block.

---

# 19. Priority P2 — Enterprise / Team Governance

Untuk mencapai product score 9.8 secara penuh dan bukan hanya technical score:

## Tambahkan optional team policy

```text
.forgeguard/policy.toml
```

atau remote signed policy.

Team dapat menetapkan:

- required modes,
- banned overrides,
- minimum coverage,
- mandatory evidence,
- critical paths,
- protected resources,
- approved tools,
- required reviewers,
- risk confirmation policy.

### Signed policy

Agent tidak boleh mengedit policy untuk melewati gate.

CI harus dapat memvalidasi:

```text
policy hash
current diff
receipt
```

---

# 20. Priority P2 — CI / PR Completion Artifact

ForgeGuard perlu menghasilkan artifact resmi:

```text
.forgeguard/reports/completion.json
```

Isi:

```json
{
  "objective": "...",
  "risk": "high",
  "worktree_hash": "...",
  "acceptance": {"covered": 8, "total": 8},
  "checks": {"passed": 12, "failed": 0},
  "runtime": {"passed": true},
  "security": {"new_blocking": 0},
  "architecture": {"violations": 0},
  "evidence_score": 98,
  "verdict": "PASS"
}
```

CI/PR kemudian memverifikasi artifact terhadap actual commit SHA.

Bukan mempercayai artifact begitu saja.

---

# 21. Priority P2 — Pre-Commit / Pre-Push Adapter

Roadmap sudah menyebut ini.

Tambahkan:

```text
forgeguard install-git-hooks
```

Tetapi sebaiknya optional.

Use cases:

- pre-commit lightweight changed scan,
- pre-push required gate,
- CI full verification.

Agent completion hook dan Git hooks menjadi defense in depth.

---

# 22. Priority P2 — Monorepo / Large Repo Intelligence

Untuk repository sangat besar, ForgeGuard harus memahami affected graph.

Contoh:

```text
changed package
   ↓
dependency graph
   ↓
affected services/packages/tests
   ↓
only relevant checks run
```

Integrasi:

- Nx
- Turborepo
- Bazel
- Cargo workspace
- Go workspace
- Gradle multi-project
- Maven multi-module

Goal:

> verification lebih akurat tanpa menjalankan seluruh dunia.

---

# 23. Priority P2 — Repository Learning Tanpa LLM

ForgeGuard bisa belajar deterministic repository conventions dari history:

- common test commands,
- directories biasanya berubah bersama,
- ownership patterns,
- historical build failures,
- public interfaces,
- migration patterns.

Bukan ML wajib.

Misalnya Git history dapat memberi signal:

```text
src/auth/token.ts berubah
→ tests/auth/token.test.ts biasanya ikut berubah
```

Jika sekarang tidak:

```text
review warning
```

Ini dapat meningkatkan “senior intuition” secara deterministic.

---

# 24. Priority P2 — AI-Specific Engineering Pack

Roadmap sudah menyebut AI tool schema, RAG, token, evaluation gates.

Ini sebaiknya menjadi diferensiasi kuat ForgeGuard.

## AI Agent / LLM Application Rules

### Tool schema

- invalid required fields,
- ambiguous tool descriptions,
- unsafe broad permissions,
- non-idempotent retries.

### RAG

- no retrieval evaluation,
- unbounded top-k,
- context overflow risk,
- missing source attribution,
- embedding dimension mismatch.

### Agent loop

- unbounded loops,
- missing max iteration,
- repeated identical tool call,
- no stop condition,
- tool retry storm.

### LLM cost

- nested model calls,
- uncontrolled parallel generation,
- huge context duplication,
- repeated system prompt expansion.

### Evaluation

- behavior changed without eval,
- eval dataset too small,
- no regression baseline,
- only happy-path prompts.

ForgeGuard punya peluang besar menjadi quality gate khusus software era agentic.

---

# 25. Real-World Benchmark Suite — Wajib untuk Claim “Best OSS”

Ini salah satu gap terbesar saat ini.

Synthetic fixtures bagus untuk regression test, tetapi tidak cukup untuk klaim real-world quality.

Buat project:

```text
forgeguard-bench
```

atau directory:

```text
benchmarks/real-world/
```

## Dataset

Minimal ratusan kasus dari repository nyata:

- JavaScript/TypeScript
- Python
- Go
- Rust
- Java/Kotlin
- mobile
- backend
- frontend
- database
- AI applications

### Label

Setiap case punya:

```yaml
repo: ...
commit: ...
issue: ...
bug_type: N+1
expected_rule: FG-DB-001
should_block: true
```

## Metrics

Publish:

- precision,
- recall,
- false positive rate,
- scan latency,
- memory usage,
- token overhead,
- completion recovery success,
- additional turns caused,
- bugs caught before completion,
- false completion prevented.

## Agent Benchmark

Ini yang paling penting.

Jalankan task yang sama:

```text
Codex without ForgeGuard
Codex + ForgeGuard
Claude without ForgeGuard
Claude + ForgeGuard
OpenCode without ForgeGuard
OpenCode + ForgeGuard
```

Measure:

- task success,
- test pass,
- hidden test pass,
- architecture violations,
- security regressions,
- unfinished requirements,
- premature completion,
- token usage,
- number of retries.

Goal claim yang sangat kuat:

> “Across N real engineering tasks, ForgeGuard reduced false completion by X% and increased verified task success from Y% to Z%.”

Ini jauh lebih bernilai daripada jumlah stars.

---

# 26. ForgeGuard Senior Engineering Standard

Untuk benar-benar menjawab target “hasil setara senior developer”, definisikan standard resmi.

Sebut misalnya:

> **ForgeGuard Senior Engineering Contract (FSEC)**

Sebuah task dianggap senior-grade jika memenuhi:

## Requirement

- objective jelas,
- acceptance criteria terukur,
- out-of-scope diketahui.

## Design

- affected component diketahui,
- architecture boundary terjaga,
- backward compatibility dinilai,
- risk dinilai.

## Implementation

- minimal necessary diff,
- tidak memperkenalkan duplicate responsibility,
- tidak mem-bypass safety.

## Testing

- relevant tests,
- negative cases,
- error cases,
- changed-code test mapping,
- coverage minimum.

## Runtime

- build works,
- application boots,
- critical path works.

## Security

- no new blocking issue,
- auth/authz verified when relevant,
- secret/data safety verified.

## Performance

- no unacceptable regression when relevant.

## Database

- migration safety,
- query behavior,
- transaction correctness.

## Delivery

- diff reviewed,
- evidence bound to final state,
- acceptance criteria covered,
- rollback documented for high risk.

Task belum boleh `DONE` sampai contract sesuai risk tier terpenuhi.

---

# 27. Recommended Risk Matrix

| Risk | Example | Minimum Verification |
|---|---|---|
| Low | copy, isolated UI styling | lint + relevant tests |
| Medium | normal API/business logic | lint + typecheck + unit + integration |
| High | auth, DB migration, payment, concurrency | full static + integration + runtime + architecture + security |
| Critical | production data, access policy, irreversible operation | full gate + rollback proof + human confirmation |

Risk classification bisa berasal dari:

- path,
- changed symbols,
- dependency files,
- security-sensitive APIs,
- migration operations,
- config,
- infrastructure,
- user declaration.

---

# 28. Recommended New Rule Families

Target rule namespace bisa berkembang menjadi:

```text
FG-ALG-*       algorithm
FG-DB-*        database
FG-NET-*       networking
FG-CON-*       concurrency
FG-CPLX-*      complexity
FG-SEC-*       security
FG-AUTH-*      authorization
FG-ERR-*       error handling
FG-COV-*       coverage
FG-DRY-*       duplication
FG-ARCH-*      architecture
FG-PARSE-*     parser reliability
FG-TEST-*      testing quality
FG-PLAN-*      task planning
FG-CONTRACT-*  API/schema contracts
FG-PERF-*      performance regression
FG-RUNTIME-*   runtime verification
FG-RISK-*      risk/rollback
FG-AI-*        AI/LLM engineering
FG-MCP-*       MCP/tool schemas
FG-DATA-*      data handling
FG-OBS-*       observability
```

---

# 29. What ForgeGuard Should NOT Become

Agar tidak kehilangan fokus:

## Jangan menjadi SonarQube clone

Gunakan external analyzers bila sudah mature.

ForgeGuard harus mengontrol **kapan analyzer wajib dijalankan dan bagaimana evidence memengaruhi completion**.

## Jangan menjadi second LLM judge by default

Ini akan:

- menaikkan biaya,
- menambah nondeterminism,
- menghilangkan keunggulan local deterministic guard.

Optional reviewer boleh ada, tapi bukan authoritative source.

## Jangan menjadi full CI platform

GitHub Actions, GitLab CI, Jenkins sudah ada.

ForgeGuard menjadi portable policy/evidence layer.

## Jangan menjadi sandbox security product

Hooks bukan OS sandbox.

ForgeGuard dapat mendeteksi dan memblokir workflow-level risk, tetapi OS permissions tetap boundary utama.

---

# 30. Product Positioning

## Positioning Lama

> Token-efficient engineering discipline for AI coding agents.

Sudah bagus, tetapi masih terlalu generik.

## Positioning yang Lebih Tajam

### Option A

> **Deterministic completion gates for AI engineering agents.**

### Option B

> **ForgeGuard turns vibe coding into verifiable engineering.**

### Option C

> **The engineering control plane between AI agents and “done”.**

### Option D

> **AI can write the code. ForgeGuard decides whether the work is actually done.**

Option D paling mudah dipahami user awam.

---

# 31. Competitive Moat

ForgeGuard tidak perlu menang karena punya paling banyak static rules.

Moat terbaik:

```text
Task contract
+ scope control
+ architecture contract
+ risk classification
+ changed-code analysis
+ test adequacy
+ runtime evidence
+ provenance
+ bounded auto-repair loop
+ final deterministic completion gate
```

Tool lain biasanya hanya punya sebagian:

```text
static analyzer
atau
merge gate
atau
secret guard
atau
AGENTS.md
atau
Stop hook
```

ForgeGuard bisa menjadi layer yang menyatukan semuanya.

---

# 32. Target Architecture 9.8

```text
                           USER INTENT
                               │
                               ▼
                    ┌─────────────────────┐
                    │  TASK CONTRACT      │
                    │ objective           │
                    │ acceptance          │
                    │ scope               │
                    │ risk                │
                    │ metric / target     │
                    └─────────┬───────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │  PLAN / DESIGN      │
                    │ affected modules    │
                    │ architecture        │
                    │ contracts           │
                    │ verification plan   │
                    └─────────┬───────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │   AI AGENT WORK     │
                    └─────────┬───────────┘
                              │
              ┌───────────────┼────────────────┐
              │               │                │
              ▼               ▼                ▼
        Static/Semantic   Test Adequacy    Risk/Security
              │               │                │
              └───────────────┼────────────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │ RUNTIME VERIFICATION│
                    │ build / boot        │
                    │ API / browser       │
                    │ DB / migration      │
                    │ perf / logs         │
                    └─────────┬───────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │ EVIDENCE RECEIPTS   │
                    │ state-bound         │
                    │ provenance          │
                    │ artifact hashes     │
                    └─────────┬───────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │ FINAL REVIEW GATE   │
                    │ acceptance coverage │
                    │ quality budget      │
                    │ architecture        │
                    │ evidence score      │
                    └─────────┬───────────┘
                              │
                     ┌────────┴────────┐
                     │                 │
                   BLOCK             PASS
                     │                 │
              bounded auto-poke      DONE
```

---

# 33. Recommended Implementation Order

## Phase 1 — “Actually Works”

Target score: **9.1–9.3**

Implement:

1. runtime verification,
2. state-bound evidence,
3. changed-function test mapping,
4. test quality checks,
5. risk classification,
6. architecture boundary rules,
7. API/schema breaking-change gate.

Ini paling besar efeknya terhadap hasil vibe coding.

---

## Phase 2 — “Senior Engineering Discipline”

Target score: **9.4–9.6**

Implement:

1. database migration safety,
2. query-plan evidence,
3. rollback proof,
4. performance regression gate,
5. security pack expansion,
6. evidence confidence score,
7. final completion artifact.

---

## Phase 3 — “Best OSS Claim”

Target score: **9.7–9.8**

Implement:

1. real-world benchmark corpus,
2. cross-agent benchmark,
3. compiler-backed semantic adapters,
4. signed/verified evidence receipts,
5. CI/PR SHA-bound verification,
6. monorepo affected graph,
7. AI/MCP engineering pack,
8. docs generated from code registry,
9. published false-positive/precision metrics.

---

# 34. Documentation and Release Hygiene

Current audit menemukan documentation drift.

Contoh:

- rule registry memiliki rule yang belum tercatat di Rule Catalog,
- beberapa docs upgrade behavior tidak sepenuhnya sinkron dengan implementation.

Untuk score 9.8, documentation tidak boleh manual duplication kalau bisa dihindari.

## Solusi

Generate docs dari code metadata:

```text
rules.rs
   ↓
forgeguard docs generate
   ↓
RULES.md
```

Capability matrix juga generated.

CLI docs juga generated dari Clap metadata.

CI:

```text
forgeguard docs check
```

jika generated output berbeda → fail.

Ini menghilangkan docs drift.

---

# 35. Definition of Done untuk ForgeGuard Sendiri

ForgeGuard harus “dogfood” ForgeGuard.

Setiap release besar idealnya harus memiliki:

- all unit/integration tests pass,
- semantic benchmark threshold pass,
- real-world benchmark threshold pass,
- installer tests pass,
- cross-platform builds pass,
- docs generated and synchronized,
- cargo audit/deny pass,
- own ForgeGuard strict gate pass,
- benchmark regression within threshold.

---

# 36. Suggested Public Benchmark Targets

Agar claim 9.8 credible, tetapkan target yang measurable.

Contoh target awal:

| Metric | Target |
|---|---:|
| Semantic precision | >= 97% |
| Semantic recall | >= 90% |
| Security precision | >= 97% |
| Security recall | >= 92% |
| False completion reduction | >= 50% |
| Verified task success lift | >= 20 percentage points |
| Added model context on pass | ~0 |
| Hook feedback | <= 2,000 chars |
| Unchanged worktree re-gate | cached |
| Critical evidence stale after code change | 100% invalidated |
| Critical risk completion without required evidence | 0 allowed |

Angka final harus berdasarkan benchmark nyata, bukan dipaksakan demi marketing.

---

# 37. Suggested Product Modes

Current `lite/default/strict` sudah bagus.

Bisa dikembangkan menjadi profile yang lebih mudah dipahami:

```text
forgeguard mode vibe
forgeguard mode standard
forgeguard mode senior
forgeguard mode critical
```

Namun jangan mengganti existing stable modes jika breaking.

Lebih aman sebagai preset:

```text
forgeguard preset senior
```

Contoh:

### Senior preset

- objective contract required,
- acceptance required,
- changed tests required,
- architecture enabled,
- runtime enabled,
- warning/error static blocking,
- evidence receipts required.

### Critical preset

Tambahan:

- rollback proof,
- human confirmation,
- full security,
- full runtime,
- signed completion artifact.

---

# 38. Recommended UX Goal

Walaupun internal system kompleks, penggunaan harus tetap sangat sederhana.

Ideal user flow:

```bash
forgeguard init
```

Setelah itu developer cukup memakai coding agent seperti biasa.

Agent integration melakukan:

```text
objective registration
plan
implementation
verification
completion gate
```

User tidak harus menjalankan 20 command secara manual.

Prinsip UX:

> **Complex inside, boring outside.**

---

# 39. What Makes ForgeGuard Better Than Skills / Prompt Files

Skill / instructions hanya mengatakan:

```text
Please inspect first.
Please test.
Please review.
```

Model masih bisa mengabaikan atau salah menilai.

ForgeGuard harus memberikan:

```text
Instruction
+ State
+ Evidence
+ Enforcement
+ Independent local decision
```

Itu positioning yang harus terus dipertahankan.

---

# 40. What Makes ForgeGuard Better Than Loop Engineering Alone

Loop engineering biasanya:

```text
implement
↓
test
↓
fix
↓
repeat
```

Tetapi tanpa objective/evidence gate, loop bisa:

- optimize metric salah,
- mengulang tanpa progress,
- memperbaiki satu test sambil merusak requirement lain,
- berhenti saat local test pass.

ForgeGuard memberikan bounded objective dan completion contract:

```text
Loop Engineering
        +
Goal Contract
        +
Scope
        +
Acceptance
        +
Evidence
        +
Runtime Verification
        +
Final Gate
```

---

# 41. What Makes ForgeGuard Better Than Traditional SAST

Traditional SAST menjawab:

> “Ada masalah di source code?”

ForgeGuard menjawab:

> “Apakah agent yang sedang mengerjakan objective ini punya bukti cukup untuk berhenti sekarang?”

Static analyzer hanyalah salah satu sensor ForgeGuard.

Sensor lain:

- test,
- compiler,
- runtime,
- browser,
- database,
- profiler,
- schema checker,
- CI,
- human approval.

ForgeGuard adalah decision layer.

---

# 42. Final Recommended Product Definition

## ForgeGuard

> **ForgeGuard is an open-source deterministic engineering control plane for AI agents. It tracks what the agent is supposed to achieve, constrains what it may touch, verifies how the code behaves, requires state-bound evidence, and blocks completion until the work meets the declared engineering standard.**

Versi singkat:

> **AI writes. ForgeGuard verifies.**

Versi marketing:

> **Turn vibe coding into verifiable engineering.**

Versi technical:

> **Deterministic completion gates for AI engineering agents.**

---

# 43. Final Recommendation

Kalau targetnya hanya membuat ForgeGuard punya scanner lebih banyak, score mungkin naik dari 8.7 menjadi sekitar 9.0.

Kalau targetnya **9.8 dan menjadi OSS terbaik untuk vibe coding**, fokus utamanya harus bergeser dari:

```text
more static rules
```

menjadi:

```text
verified engineering completion
```

Urutan investasi paling penting:

1. **Runtime Verification**
2. **State-Bound Evidence Receipts**
3. **Test Adequacy + Changed-Function Test Mapping**
4. **Architecture Contract**
5. **Automatic Risk Classification**
6. **API / Schema Compatibility Gate**
7. **Database Migration + Query Plan Guard**
8. **Rollback Proof for High-Risk Changes**
9. **Performance Regression Gate**
10. **Real-World Agent Benchmark Suite**
11. **Compiler-Backed Semantic Adapters**
12. **SHA-Bound CI/PR Completion Artifact**
13. **AI/MCP Engineering Pack**
14. **Generated Documentation / No Docs Drift**

Jika bagian-bagian ini selesai dengan benchmark nyata, ForgeGuard tidak lagi hanya menjadi “guardrail tambahan untuk Claude/Codex”.

ForgeGuard bisa menjadi:

> **the open-source quality control layer that every AI engineering agent can sit behind.**

Dan itu adalah kategori yang jauh lebih defensible daripada mencoba mengalahkan SonarQube atau Semgrep di permainan mereka sendiri.

---

# 44. Target North Star

North star yang disarankan:

> **An AI agent must never be allowed to say “done” merely because it believes it is done. It must prove the result against the objective, final code state, runtime behavior, and engineering policy.**

Dalam bahasa sederhana:

> **Agent boleh pintar atau bodoh. ForgeGuard harus membuat hasil akhirnya tetap bisa dipercaya.**

