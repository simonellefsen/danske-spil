---
type: wiki-schema
tags:
  - danske-spil/wiki
  - maintained-by-llm
updated: 2026-05-25
---

# Wiki Schema

This file is the operating contract for agents maintaining the Danske Spil knowledge wiki.

## Layer Rules

- Raw sources are immutable. Read, cite, and summarize them, but do not rewrite them as wiki maintenance.
- The wiki is generated synthesis. Agents may create and update pages under `wiki/`.
- Browser observations must be sanitized before they are committed.
- The schema defines conventions. Update it when the workflow changes.

## Page Types

Use YAML frontmatter on maintained wiki pages:

```yaml
---
type: concept
tags:
  - danske-spil/wiki
updated: 2026-05-25
sources:
  - wiki/sources/danske-spil-netspil-terms.md
---
```

Recommended `type` values:

- `wiki-index`
- `wiki-log`
- `wiki-schema`
- `source-note`
- `concept`
- `runbook`
- `decision`
- `experiment`
- `capability`

## Link Rules

- Use relative Markdown links for wiki-to-wiki links.
- Use Markdown links with absolute local paths for repository files outside `wiki/`.
- Prefer linking source files and docs over copying long excerpts.
- Avoid unresolved wikilinks unless the missing page is intentionally listed as an open task.

## Ingest Workflow

1. Read the source.
2. Create or update `wiki/sources/<source-name>.md`.
3. Update affected concept, runbook, decision, or experiment pages.
4. Update `wiki/index.md`.
5. Append one entry to `wiki/log.md`.
6. If the source changes safety or betting behavior, cross-link [docs/compliance-and-safety.md](/Users/lindau/codex/danske-spil/docs/compliance-and-safety.md).

## Query Workflow

1. Search `wiki/index.md` first.
2. Use `rtk qmd search` or `rtk qmd query` when the local index is configured.
3. Retrieve full pages before making claims.
4. Cite wiki pages and raw source files.
5. If the answer creates durable knowledge, update the relevant wiki page and log it.

## Safety Rules

- Never store Danske Spil credentials, cookies, MitID data, Spil-ID identifiers, payment data, TOTP seeds, API keys, database passwords, or raw account payloads in the wiki.
- Browser automation claims must point to source notes, screenshots, or code.
- Strategy learning pages must distinguish hypothesis, evidence, result, and active baseline.
- Hermes may propose changes; only approved code/config/database flows may promote changes.
- Real-money placement is disabled until a decision record explicitly changes that posture.
