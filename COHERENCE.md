# Product Coherence for Pmacs

## Purpose

Pmacs already has an unusually strong technical foundation for an editor at its
stage of development. Its daemon/frontend split, semantic rendering protocol,
CRDT-based editing, structured worker runtime, Lua programmability, language
tooling, package resolver, terminal support, and remote-capable architecture all
point toward a system with genuine long-term differentiation.

The next challenge is not primarily adding more isolated capabilities. It is
making the existing and planned capabilities converge into a coherent product.

Visual Studio Code is used throughout this document as a reference point because
it is an exceptionally successful modern editor. Pmacs is obviously not trying
to become VS Code. Its goals are substantially different: live programmability,
inspectability, stronger concurrency semantics, frontend plurality, and deeper
user control are central to Pmacs in ways they are not central to VS Code. The
useful lesson is therefore not to copy VS Code's interface or architecture
wholesale, but to understand how a technically complex system can become
immediately useful, progressively discoverable, and easy to adopt.

The deeper reference point is Emacs. Emacs's beauty comes from its ontological
unity: the editor is text, Lisp, commands, buffers, and a running system that
the user can interrogate and change. Its enduring achievement is not any single
feature, but that it created the kind of environment in which generations of
users could build almost anything.

Pmacs should preserve that unity while correcting the accidental historical
constraints beneath it: cooperative rather than general parallelism, unclear
ownership, global mutation, difficult unloading, rendering coupled too closely
to the core, opaque latency, implicit remote context, and inconsistent package
lifecycle.

Pmacs does not need to contain everything Emacs contains before it can be
considered a successor. It must instead remain the kind of system in which
everything Emacs contains could eventually be built — with clearer ownership,
stronger concurrency, richer frontends, explicit execution locations, and fewer
historical traps.

That places the VS Code comparison in its proper role. VS Code demonstrates how
a complex development environment can be coherent, approachable, and immediately
useful. Emacs demonstrates how an editor can become a live, fertile,
user-transformable world. Pmacs should combine the adoption discipline of the
former with the programmability and unity of the latter.

The core product objective should be:

> **Pmacs should be immediately excellent, progressively understandable,
> completely inspectable, and ultimately replaceable.**

A user should receive a polished workstation before they become an editor
engineer. If they choose to become one, the entire system should remain open to
them.

## 1. The Product Problem

Pmacs is building many difficult things correctly and in parallel. That is
appropriate for an early systems project. The risk is that the project succeeds
architecturally while remaining fragmented experientially.

A technically sophisticated editor can still feel incoherent when:

- installation requires repository knowledge;
- capabilities exist but are difficult to discover;
- subsystems expose unrelated interaction conventions;
- project, process, terminal, language-server, and remote state are modeled
  separately;
- configuration is powerful but provenance is unclear;
- packages can extend the editor but cannot be understood, controlled, or
  attributed;
- background work is concurrent but not meaningfully owned;
- new users must configure the system before they can experience its strengths.

The relevant distinction is between **capability completeness** and **product
coherence**. Capability completeness asks "can pmacs do X?". Product coherence
asks whether a user naturally encounters X at the right time, whether X behaves
by shared conventions, whether the user can understand why X is active, and
whether X feels like part of one editor rather than an adjacent demonstration.

Pmacs is well on its way toward capability completeness in several major areas.
Product coherence must now become an explicit development track rather than an
emergent consequence of subsystem work. Audits have found all eight points above
true of pmacs, with three structural causes: substrate without surface (a
mechanism exists and nothing in the product reaches it), the silence asymmetry
(success is announced and failure is not), and coherence debt that compounds
because no arc is asked to state its coherence impact.

## 2. The Golden Product Journey

Pmacs should maintain one protected end-to-end experience against which all
major work is tested:

1. Install Pmacs.
2. Launch it without prior configuration.
3. Open a real project.
4. Understand the visible interface.
5. Edit immediately.
6. Receive language intelligence.
7. Find a symbol or file.
8. Open a terminal.
9. Build or test the project.
10. Inspect and act on an error.
11. Understand what background work is running.
12. Close and later restore the workspace.

This does not need to exercise every advanced feature. It exists to prove that
the editor's components form a usable whole, and it should be a release gate:
new architectural work is evaluated partly by whether it improves, preserves, or
complicates the journey. A strong initial target is a Rust project, because Rust
stresses many of pmacs's intended strengths: project detection, toolchain
discovery, language-server lifecycle, async diagnostics, build and test
integration, terminal use, large compilation workloads, symbol search,
background indexing, structured error presentation.

## 3. A Strong Zero-Configuration State

Pmacs should not require configuration before it becomes pleasant. The default
experience should demonstrate the editor's thesis: responsive editing, visible
asynchronous work, coherent project awareness, language intelligence, integrated
terminal and task execution, helpful diagnostics, discoverable commands,
graceful failure when external tools are absent.

Configuration should be an escalation path:

1. The editor works.
2. The user notices a preference.
3. The relevant setting or command is easy to find.
4. The user changes it.
5. The editor explains where the effective value came from.
6. Advanced users can replace the behavior entirely.

### Recommended default surface

The graphical frontend should have a deliberate default workspace with a
restrained number of visible regions: main editor area; compact statusline;
optional project/files surface; bottom panel for terminal, build output,
diagnostics, and other transient tools; command palette; contextual actions;
unobtrusive background activity indicator. The TUI should express the same
conceptual model within terminal constraints. The goal is not identical geometry
across frontends — it is shared nouns, commands, lifecycle, and state.

## 4. Progressive Disclosure

Pmacs should support several levels of use without requiring users to inhabit
the most advanced one. These levels should be different presentations of the
same underlying objects — a command selected from a context menu, invoked
through `M-x`, bound to a key, called from Lua, or triggered by an agent should
be the same command object.

## 5. Unify Discoverability

Pmacs already has the beginnings of a strong command registry. This should
become the center of a broader discovery model. Every meaningful action should
eventually expose a stable symbolic identity, title, description, category,
aliases, current keybindings, provenance, an applicability predicate with an
explanation when unavailable, an argument schema, destructive, asynchronous and
reversible flags, locality, related commands and settings, and a source
location; settings, packages and workers expose the analogous sets. The general
principle:

> **Anything that can affect the user should be discoverable as a
> structured object with identity, provenance, ownership, and
> lifecycle.**

## 6. Eliminate Hardcoded Interaction Islands

Pmacs's public programmability story will be strongest when all major
interaction layers pass through ordinary registries and extension points.
Temporary or modal interfaces — incremental search, query replace, minibuffer
prompts, completion menus, context menus, transient selectors — should
eventually use inspectable keymap layers rather than special Rust-level
interception. A general transient keymap model includes priority, activation
condition, owner, lifetime, fallback behavior, discoverability, help labels, and
cancellation behavior.

## 7. First-Class Workspaces

Project-root detection is useful, but pmacs needs a richer workspace object. A
project answers "which root contains this file?"; a workspace answers "which
persistent development environment owns this set of activity?" A workspace
should eventually own:

This matters for multi-root language servers, monorepos, generated files, remote
projects, containers, HPC environments, per-project packages, task ownership,
session restoration, and project-specific trust. The workspace should be a core
runtime entity, not an informal convention shared across unrelated subsystems.

## 8. First-Class Execution Locations

Pmacs's daemon/frontend architecture gives it an excellent basis for remote
development. The next step is to model execution location explicitly — a value
that can be inspected and assigned, not an implementation detail hidden inside
file access or process spawning:

Filesystem roots, processes, terminals, language servers, workers, debuggers,
indexers, package services, and build/test tasks should all carry a location.
That makes answerable: where is this server running? where will this build
execute? is this terminal local? can this worker migrate? what happens if the
remote daemon disconnects?

## 9. Extend the Worker Model into Structured Concurrency

Pmacs's worker system is one of its most distinctive strengths. Cancellation,
supersession, streaming, frame-aware draining, and the `*workers*` view provide
a strong basis. The next step is ownership and hierarchy: every substantial task
should have an owner, a workspace, an optional buffer or view, a parent,
children, a latency class, a cancellation scope, a resource budget, an execution
location, progress, and failure attribution. Cancelling a command should cancel
its children; closing a workspace should terminate or detach workspace-owned
work; reloading a package should stop package-owned tasks. The activity view
should answer what is running, why, who owns it, where, what depends on it, and
what cancellation will affect. That turns parallelism into a product feature
rather than an implementation claim.

## 10. Define Extension Trust and Isolation Classes

Pmacs should preserve live, low-friction programmability — it should not force
all extensions into rigid out-of-process APIs. At the same time, namespace
isolation inside a shared Lua state is not enough for fault containment,
security, latency containment, memory accounting, native-code isolation,
reliable unloading, or project-local trust. Pmacs should define extension
classes before the ecosystem becomes large:

trusted core packages (in-process, deep API access, distributed with pmacs or
explicitly trusted); normal Lua packages (shared, managed runtime, declared
capabilities, owned registrations and workers, execution budgets, measurable
latency, reloadable lifecycle, package-level error attribution); isolated
service extensions (separate process, typed RPC, crash recovery, resource
accounting, explicit filesystem, process and network permissions); and
project-local or untrusted extensions (explicit approval, restricted
capabilities, strong isolation, workspace-scoped trust, easy revocation).

## 11. Configuration as Typed, Layered Data

Pmacs's typed configuration registry is the correct foundation. It should
develop into a layered system with explicit provenance — built-in defaults,
profile defaults, user settings, machine-local, remote-location, workspace, root
or folder, language or mode, buffer-local, session overrides — and a setting
inspection view should show the full chain and the active source. Pmacs should
also preserve three distinct levels — **settings** (typed declarative data),
**behavioral customization** (commands, hooks, keymaps, Lua), **package
construction** (new capabilities) — so that users do not need executable Lua for
ordinary preferences, while advanced users can still replace the mechanism.

## 12. Profiles as Product-Level Bundles

Pmacs should offer a small number of official profiles bundling default keymaps,
visible interface regions, package recommendations, settings, task conventions,
discovery hints, and onboarding: **Pmacs Standard** (an approachable graphical
workstation), **Emacs** (familiar bindings, minibuffer-centered), **Minimal**,
and later **Research Workstation** (terminals, remote machines, Slurm, proof
assistants, long-running builds). Profiles must not create separate products;
they exercise the same registries and primitives.

## 13. Package Experience, Not Merely Package Resolution

Pmacs already has serious package-resolution machinery. Product coherence
requires a package *lifecycle* experience: search, installation, updates,
disable, reload, uninstall, version inspection, dependency graph, compatibility
warnings, capability declarations, ownership inspection, error history,
active-worker inspection, resource use, trust state. Installation should work
during a running session, and users should be able to install coherent
capability bundles ("Rust Development") rather than individual packages.
Marketplace sequencing: stable format, ownership and reload lifecycle, in-editor
manager, curated registry, bundles, publisher identity, public marketplace.

## 14. Coherent Workbench Primitives

Pmacs should resist implementing each subsystem with a custom UI vocabulary. It
should provide a small set of reusable view primitives — editable text view,
virtual list, tree, structured table, inspector, output channel, diagnostics
collection, task and progress view, diff view, transient selector, contextual
popup, side panel, bottom panel, help view — and packages should provide
structured models to them. Git status, project files, symbol outlines, package
dependencies, and worker trees should share one tree model with consistent
selection, expansion, filtering, action discovery, mouse and keyboard behavior,
persistence, and accessibility.

## 15. Contextual Affordances

Pmacs should remain excellent for keyboard-driven users while making
capabilities visible to users who do not know their names: a diagnostic should
offer code actions; a test definition run/debug; a Git change stage/revert/diff;
a missing formatter configuration guidance; a symbol
references/rename/definition/documentation; a remote workspace its location; a
long-running task progress and cancellation. Affordances should invoke ordinary
commands, never separate logic paths.

## 16. Productize the Semantic Frontend Architecture

The semantic protocol should be visible as a product advantage: native frontend
rendering, frontend-specific typography, high-quality decorations, efficient
incremental updates, accessible semantic information, multiple simultaneous
frontends, stable remote attachment, frontend experimentation without
reimplementing editor semantics. To preserve coherence: core commands stay
frontend-neutral, semantic identities stay stable, capabilities are negotiated
explicitly, degradation is graceful, layout state is separated from semantic
state, and no frontend becomes the de facto privileged implementation.

## 17. Distribution Is Part of the Product

Pmacs should eventually be installable without repository familiarity:
reproducible release builds, Linux and macOS binaries, checksums and signatures,
stable and nightly channels, one-command update, rollback, protocol- and
package-API compatibility reporting. First launch should create/locate config
directories, explain the default profile, identify optional external tools, and
let the user open a project immediately.

## 18. Onboarding

Pmacs needs onboarding that teaches concepts through use: open a project, the
command palette, find a file, a terminal, inspect a diagnostic, view workers,
change a setting and inspect where it came from, the Lua REPL, redefine a
command. That sequence communicates the whole thesis — already useful,
discoverable, visible computation, explainable settings, programmable internals
— and it should be an ordinary, restartable help workspace, not a one-time modal
wizard.

## 19. Product Coherence Acceptance Tests

Pmacs should add acceptance tests that exercise product behavior across
subsystems, complementing (not replacing) subsystem tests: installation and
first launch (no config, open a directory, usable workspace, actionable guidance
for missing tools); command discovery (search by title and synonym; display
keybinding, provenance and availability; invoke from palette and menu through
the same object); workspace lifecycle (multi-root open, servers, terminal,
build, close, restore, ownership cleanup); worker ownership (start completion,
search or build, inspect, cancel a parent, confirm child cancellation and UI
recovery); package lifecycle (install in-session, inspect contributions,
disable, confirm disappearance, reload, uninstall cleanly); remote execution
(attach to a remote daemon, edit optimistically, remote terminal and server,
disconnect and reconnect, coherent state).

## 20. Recommended Priority Order

Position is the schedule. The order ranks the concerns above; the measured state
of each lives outside this file.

1. **Protect the golden product journey** (§2): a release gate; its acceptance
   suite is the ratchet, and stages add rows, none removes them.
2. **Make workspace and location explicit** (§7, §8): the long-lead model arc; a
   session restore or a project sidebar is the signal to start it.
3. **Strengthen extension ownership and isolation** (§10, §13): owner-carrying
   registrations, complete removal APIs, attribution by default.
4. **Unify discovery** (§5): wiring over existing introspection, richer `M-x`
   rows, title and category on commands, help unification.
5. **Finish the workbench convergence** (§14): adoption of the shared
   primitives, then table, inspector and diff views.
6. **Productize configuration** (§11): provenance, layering, adoption migration,
   persistence.
7. **Build package lifecycle UX** (§13), after 3.
8. **Ship binaries and release channels** (§17): each increment past binaries on
   tag is a decision, not a continuation.

A standing process rule accompanies all of them: **every framing or task row
that affects coherence states its coherence impact** — which journey steps it
touches, whether it adds an interaction island, whether its options enter the
config registry, whether its background work is attributed — so the debt stops
compounding silently.

## 22. What Pmacs Should Preserve and Deepen

Pmacs should not trade away the qualities that justify its existence: live
programmability (redefine and unregister at runtime, per-package environments);
implementation inspectability (a source location on every registration,
mandatory descriptions); replaceable interaction models; multiple genuine
frontends over semantic rendering; explicit parallel work with cancellability
and observability; the remote daemon architecture; user control over the editor
as a running system. The goal is not to make pmacs less powerful so that it
becomes approachable; it is to make power **progressively available**.

## 23. Product Thesis

Emacs offers: *the editor is a programmable environment, and the user may
transform it completely.* VS Code offers: *the editor is already a coherent
development workstation, and extensions fill in the remaining gaps.* Pmacs
should offer:

> **The editor is already an excellent workstation, and every part of
> that workstation remains inspectable, programmable, concurrent, and
> replaceable.**

Its strongest distinctive proposition is not "Emacs in Rust" or "Emacs with
threads":

> **Pmacs is a live-programmable editor in which computation,
> interfaces, ownership, and execution locations are explicit — allowing
> local, remote, interactive, and background work to coexist without
> freezing or becoming opaque.**

Of the two halves of that sentence, "without freezing" is delivered and "without
becoming opaque" is not yet true: for the failures a new user meets first (§1),
for background work (§9), for settings (§11), and for what a key will do while a
modal surface is active (§6). Product coherence is what makes the architecture
perceptible: without it pmacs is an impressive collection of subsystems, with it
a workstation whose complexity is available without being imposed.
