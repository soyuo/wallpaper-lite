# Agent workflow

## Work planning

1. Before changing product code, inspect the open issues and the Phase 0–8 milestones in order. Work on the earliest incomplete phase; do not begin later-phase work while an earlier phase still has unmet acceptance criteria.
2. Before implementation, create or select an issue that states the goal, observable acceptance criteria, ordered implementation steps, dependencies, and scope boundaries. Issue and milestone administration is workflow work; it does not need a parent issue, branch, commit, or pull request.
3. Create a branch from the default branch using `type/<issue-number>-<short-kebab-case-description>`. Allowed types: `feature`, `refactor`, `fix`, `docs`, `test`, `chore`.
4. Inspect the current code and repository documentation before choosing files or verification commands. The QLM product documents are design inputs; confirm that implementation paths and behavior still match the repository before relying on them.

## Phase sequencing

- If the earliest incomplete phase has an actionable issue, take the earliest unblocked issue. Check for duplicates first.
- If that phase has no actionable issue and no external blocker, decompose only its remaining scope into the smallest independently reviewable issues that together satisfy its acceptance criteria. Do not bootstrap issues for later phases.
- Assign each implementation issue to exactly one phase milestone and exactly one work-type label. Add one priority label after triage; add area, status, or contributor labels only when useful.
- An empty issue queue does not mean a phase is complete. Verify its acceptance criteria against the implementation and create issues for any remaining in-scope work.
- If all remaining work in the earliest incomplete phase is externally blocked, report the blocker instead of skipping ahead. If work does not fit a phase, leave it unassigned for triage.
- Use the matching issue template: `feature.md` for new product behavior, `fix.md` for resolving a confirmed defect, `refactor.md` for restructuring, `documentation.md` for docs-only work, `task.md` for other planned work, and `bug_report.md` to report a defect.

## Roadmap milestones

| Milestone | Scope |
| --- | --- |
| Phase 0: 개발 기반 구축 | Rust workspace, Tauri 2 + React/TypeScript, crate boundaries, IPC, logging/error model, Windows API and D3D11 bootstrap |
| Phase 1: Windows Desktop Host | Progman/WorkerW validation, wallpaper window behind icons, Explorer restart detection and cleanup |
| Phase 2: 이미지 렌더링 | WIC decode, D3D11 image rendering, fit modes, transforms and color shaders |
| Phase 3: 비디오 렌더링 | Media Foundation, bounded decode queue, PTS sync, FPS and transport controls |
| Phase 4: 라이브러리와 편집기 | Media library, UI pages, non-destructive editing, preview/apply/cancel and undo/redo |
| Phase 5: 고급 재생 제어 | Timeline, segment looping, speed, volume and persisted playback settings |
| Phase 6: 멀티 모니터 | Per-monitor settings, Independent/Duplicate/Span, hotplug and DPI/resolution changes |
| Phase 7: 성능·안정성·복구 | Fullscreen/battery policies, resource use, error isolation, settings recovery and long-run stability |
| Phase 8: 제품 완성 및 릴리스 | Product UI finish, tray/autostart, installer/uninstaller, Windows 10/11 acceptance and release |

Milestone assignment must match the listed scope and the milestone's remote acceptance criteria. Do not pull post-release ideas into these phases.

## Labels

Apply exactly one work-type label to each issue:

| Label | Use |
| --- | --- |
| `bug` | Report a defect |
| `feature` | New user-visible capability |
| `fix` | Implementation for a confirmed defect |
| `refactor` | Internal restructuring without intended behavior change |
| `task` | Planned test, CI, tooling, or repository work |
| `documentation` | Documentation-only work |

Apply one priority label after triage: `priority: critical` (data loss, security, or release blocker), `priority: high` (core flow blocked), `priority: medium` (normal planned work), or `priority: low` (polish/optional).

Area labels may be added in addition to the work-type and priority labels: `area: ui`, `area: host`, `area: renderer`, `area: media`, `area: editor`, `area: playback`, `area: display`, `area: performance`, `area: ipc`, `area: config`, `area: installer`, `area: tests`, `area: ci`, and `area: docs`. Use `status: blocked` only while an external prerequisite prevents progress; remove it when unblocked. `help wanted`, `good first issue`, `duplicate`, `wontfix`, and `accessibility` are optional triage labels.

## Commits and pull requests

- Use `type(scope): Summary` for commit subjects and pull request titles. Allowed types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`.
- Allowed scopes: `project`, `host`, `renderer`, `media`, `editor`, `playback`, `display`, `performance`, `ipc`, `config`, `ui`, `test`, `ci`, `release`, `docs`.
- Keep each commit focused on one coherent change. Do not include unrelated cleanup.
- Link the implementation issue from the pull request. Request an independent diff review and address actionable findings before merge.
- Check configured CI to completion. If checks are not configured or could not run, state that plainly and use the issue's acceptance criteria as the review gate. Merge only after review and required checks pass.

## Product invariants

- Never modify, move, or delete a user's source image or video as part of wallpaper editing. Keep editor changes non-destructive and make preview and desktop rendering use the same transform rules.
- Keep UI and wallpaper host responsibilities isolated. Contain media, monitor, Explorer, IPC, and GPU failures so one failing monitor or component does not unnecessarily take down the rest of the application.
- Keep decode queues bounded and synchronize video frames using presentation timestamps.
- Validate the desktop host window before attaching it to the desktop and clean up windows/resources on shutdown and Explorer restart.
- Persist settings with validation and recovery. Do not claim atomicity, backup, or restore behavior unless the implementation provides it.
- Treat paths, monitor identities, DPI/resolution changes, fullscreen detection, sleep/resume, and GPU device loss as correctness concerns when touching those areas.
- The planned layout is `apps/ui`, Rust crates for `core`, `desktop`, `renderer`, `media`, `display`, `performance`, and `ipc`, plus `host`, `shaders`, `tests`, and `docs`. This is a target from the design documents, not proof those paths already exist.

## Verification

- Inspect available scripts and toolchain requirements before selecting checks. Run focused checks for changed behavior and record the exact command and result in the pull request.
- Do not claim tests, Windows behavior, GPU acceleration, or long-duration stability were verified unless they were actually exercised in a suitable environment.
- For rendering or media changes, include the relevant image/video format, resolution, FPS, monitor arrangement, and Windows version in verification notes when available.
