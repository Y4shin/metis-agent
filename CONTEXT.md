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
complexity is added through plugins. A plugin supplies tools through
capabilities; it does not export workflow steps.
_Avoid_: extension, integration

**Bundle**:
The one file a plugin is installed as: a `.metis` archive holding the manifest
and one library per target, inside a signed and versioned container. Signing
covers everything a user installs, so there is no second artifact to keep in
step.
_Avoid_: package, plugin file, archive (for the shipped artifact)

**Manifest**:
The plain-text file at a bundle's root. It declares the plugin's identity and
version, the ABI major and the minimum minor it needs, its library path per
target, its capabilities with their schemas, and its configuration schema. It is
written by hand or generated from the same source as the plugin's own
declaration, and the host cross-checks the two.
_Avoid_: metadata, descriptor (for the file)

**Capability**:
One named thing a plugin offers, qualified as `<instance>.<tool>` and carrying a
kind, a version, and request and response schemas. A plugin advertises its
capabilities before it serves anything, and the host refuses a call to one the
plugin does not declare.
_Avoid_: endpoint, command, tool (alone)

**Target**:
An operating system and architecture pair, spelled `<os>-<arch>`, such as
`linux-x86_64`. A bundle carries one library per target it supports, and the host
loads the one that matches it.
_Avoid_: platform (alone), build

**Host context**:
What the host tells a plugin about itself when it initialises it, delivered in the
same payload as the plugin's configuration. It names the instance by the identity
the host resolved that configuration under, and is permissive so a host may add a
field.
_Avoid_: environment, settings

**Disabled plugin**:
A plugin the host refused to start. It is still listed, with its name, the reason
and its capabilities when a manifest was readable, and invoking one answers the
disabled route error with that reason. A disabled plugin whose manifest could not
be read is named by its bundle's file stem.
_Avoid_: failed plugin, broken plugin

**Sink**:
The single destination for everything a plugin says outside its response:
progress, events, logs and service requests. It cannot fail and must not block,
and a host with no sink is legal.
_Avoid_: callback handler, output stream

**Ordering ledger**:
The host's ordered record of the lifecycle steps it took for an instance, from
load through call, cancellation and teardown. It is what makes unloading safe to
check: an instance is unloaded only when nothing is inside plugin code.
_Avoid_: log, history, audit trail

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
