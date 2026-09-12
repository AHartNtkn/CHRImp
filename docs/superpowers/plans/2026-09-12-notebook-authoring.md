# Notebook authoring implementation plan

Goal: complete the notebook's basic document, authoring, navigation and export workflows.

Architecture: a versioned JSON notebook owns a program, named queries and graph positions. The editor projects the selected query into the existing execution protocol. Runtime execution/recovery remains separate from portable authoring documents. Browser file pickers and downloads provide file operations; existing IndexedDB recovery continues to protect the working document.

1. Implement document validation and round-trip serialization; New, Open, Save, Save As, title and unsaved-file state. Validate before replacing the working document. Preserve the working document on cancellation or invalid input.
2. Add named query creation, switching, renaming, duplication and deletion. Keep each query's graph positions with it and use the selected query for execution.
3. Add fragment/rule copy, cut, paste and duplication with fresh variable identities on paste while preserving internal sharing; implement multi-selection, group movement and deletion. Preserve port order.
4. Add rule reordering and search results that navigate to rules and relations. Persist layouts across reload and notebook file round trips.
5. Add complete SVG diagram export and structured answer export. Add standard keyboard shortcuts, respecting native text editing.
6. Verify pure transformations, native browser document/file workflows, multi-canvas editing and disposal, query switching and execution. Update concise usage notes and commit completed work.

Validation follows the affected behavior. No engine semantics changes or new dependencies are planned.

Implementation complete. All nine JavaScript test files pass; the offline Rust build passes. Native browser checks cover storage/recovery, independent canvases, routing and export; live authoring checks cover naming, fragments, keyboard undo/duplication, rule ordering and query execution. File operations have direct serialization, round-trip, cancellation and failure tests. Native OS picker interaction remains unverified because the available browser controls could not exercise those dialogs.
