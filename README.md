# Metis Agent

> A minimal, extensible personal AI assistant that runs as a persistent daemon on your own computer or a server.

Metis Agent is a minimal, extensible personal AI assistant. It runs as a persistent daemon on your own computer or a server, making it reachable across devices and able to carry out work beyond an individual chat session. It emphasizes simplicity, plugin-based customization, and efficient prompt and context usage, so that it works well with smaller local models.

Chat sessions are supported. The point is not that Metis replaces chat, but that the harness around it stays simple.

## Why I built this

I built Metis for myself after trying OpenClaw and Hermes. Both are capable, but they felt too heavyweight for what I actually do: simple, purpose-specific tasks. Their system prompts are large, their context and tool machinery assumes a capable model, and much of that cost is paid even when the task itself is small. That is an awkward fit when the goal is to use a small local model for a focused job.

Metis is the smaller alternative for those cases.

## What it emphasizes

- **A persistent daemon.** Metis runs as a long-lived process on your own computer or a server, so it is reachable across devices and not confined to a single chat session.
- **Simplicity.** The core stays small, with as little as possible baked in.
- **Plugin-based customization.** Capabilities are added through plugins, so you can assemble the assistant you actually want.
- **Efficient prompt and context usage.** Lean prompts and context keep smaller local models on-task instead of drowning them.
- **Chat sessions.** Supported, with a simple harness around them.

## Status

Metis is at an early stage. This document describes the motivation and the goals; it deliberately does not document an architecture, an installation procedure, or a plugin API, because those are not settled yet. Whether things like sessions and tasks exist, and in what form, is still up for debate. Chat channels are not planned at this point.

## License

Metis Agent is licensed under the [GNU Affero General Public License v3.0](https://www.gnu.org/licenses/agpl-3.0.html) (AGPL-3.0). If you run a modified version of Metis as a network service, the AGPL requires you to make the corresponding source available to its users.
