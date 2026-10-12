# Metis Agent

A minimal, extensible personal AI assistant that runs as a persistent daemon
on your own computer or a server.

## Language

**Metis Agent**:
The assistant itself, and the name of this repository. It is minimal and
extensible: the core stays small, and capability is added rather than baked
in.
_Avoid_: metis (as prose), the bot

**Daemon**:
The long-lived process at the center of Metis. It runs on your own computer
or a server, stays reachable across devices, and is not confined to a single
chat session. The daemon is what makes Metis a service you own rather than a
window you visit.
_Avoid_: server (that is a deployment target, not the process), backend

**Plugin**:
An add-on that supplies a capability to the daemon. Plugin-based
customization is how Metis stays minimal: the core is small, and optional
complexity is added through plugins. The set of plugin types is not settled.
_Avoid_: extension, integration

**Chat session**:
A conversation with the assistant. Supported, with a deliberately simple
harness around it. Whether sessions become a first-class persisted concept
is still open.
_Avoid_: thread, conversation

**Small local model**:
A model that runs on modest local hardware and has a small context window
relative to hosted frontier models. Metis is designed to work well with
these, which is why efficient prompt and context usage matters.
_Avoid_: tiny model, weak model

## Still open

This vocabulary is young. Whether concepts such as sessions and tasks exist
as first-class things, and in what form, is still up for debate. Add terms
here as they settle rather than guessing them in.
