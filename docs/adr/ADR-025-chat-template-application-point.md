# ADR-025: The serving executor applies the chat template

- Status: accepted (2026-10-06)
- Owners: Windows Product Engineer (desktop caller), Runtime Engineer
  (executor), Protocol Architect (§6.2 semantics)
- Reviewers: Security (prompt-flow unchanged), Test and Release (regression
  tests below)

## Context

msp-v1 §6.2 says clients send plain `{role, content}` messages and "the
serving runtime applies the chat template". Two implementations disagreed
with that sentence in practice:

1. The local executor (`SingleLocalExecutor`) flattened message contents
   with newlines and passed the role-less text to llama-server's
   `/v1/completions` — an endpoint that applies **no** template (only
   `/chat/completions` does). Tiny models answered role-less text with
   immediate EOS: the v0.2.11 "0-token replies" quirk.
2. The desktop chat box pre-rendered ChatML client-side and stuffed the
   whole conversation into one `user` message — working, but it made the
   desktop the template owner and would double-template the moment any
   other layer also templated.

The exact-token path (ADR-022 vocab recovery via `/v1/completions`
`logprobs.bytes`) requires the raw endpoint, so "just use
/chat/completions" is not available for swarm serving.

## Decision

1. **The serving executor owns template application, exactly once.**
   `SingleLocalExecutor::execute` renders the conversation in ChatML
   (`modelswarm-node::executor::render_chatml`) before tokenizing.
   Clients — the desktop included — send plain `{role, content}` turns.
   The desktop's client-side `render_chatml` is removed.
2. **ChatML is the catalog's template family, by evidence.** Every
   promoted profile (Qwen2.5/Qwen3/Qwen3.5/Qwen3.8, SmolLM2) uses
   ChatML-special-token templates (their `chat_template_hash` inputs are
   ChatML-shaped). Rendering ChatML server-side is correct for the whole
   current catalog and fixes the 0-token quirk at its root.
3. **Non-ChatML profiles are gated on template metadata.** A future
   Llama-style profile must not silently receive ChatML. Adding such a
   profile requires either (a) a template-family field in the manifest
   (schema change → new ADR), or (b) shipping the profile's template
   renderer. Until then the resolver/protocol treat "ChatML-family" as a
   catalog invariant.
4. **The cooperative executor follows when it ungates.**
   `SpeculativeExecutor` (modelswarm-session) still flattens; cooperative
   phases are ADR-gated anyway and must adopt this rule (or per-profile
   template metadata) as part of their own ADR.

## Consequences

- `/v1/chat/completions` against a hosted node now produces templated
  prompts for every profile: role markers, generation prompt
  (`<|im_start|>assistant\n`), one template application end to end.
- The 0-token-reply quirk on tiny models is fixed at the executor, not
  papered over in any single client.
- Regression locks: `executor::tests::chatml_rendering_marks_roles_and_opens_assistant_turn`
  pins the exact rendering; the desktop no longer ships a template of its
  own (dead code removed).
- Prompt-privacy posture is unchanged: the rendered template is
  tokenized and passed to the local runtime only; no layer logs content.
