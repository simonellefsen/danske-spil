---
type: source-note
tags:
  - danske-spil/wiki
  - project-knowledge
  - source
updated: 2026-05-25
sources:
  - https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f
---

# LLM Wiki Pattern

This project uses an LLM-maintained wiki pattern.

The operating idea is simple: the wiki is a maintained synthesis layer that lets future Codex and Hermes sessions reuse durable project knowledge instead of rediscovering it from raw files and chat history.

## Local Adaptation

For this repository, the wiki should preserve:

- Browser investigation findings.
- Danske Spil safety and terms lessons.
- Oddset and Tips navigation models.
- Strategy experiments and simulation results.
- Kubernetes and Hermes operating procedures.
- Decisions about whether the project remains observe-only or moves toward human-approved action.

The wiki must not become a control plane. It explains and links; it does not approve bets or store secrets.
