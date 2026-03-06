# mybot roadmap

## High priority

- [ ] Add `/redo` to complement `/undo`
- [ ] Improve rollback coverage for `run_command` side effects
- [ ] Add dedicated custom tool management UI
- [ ] Add stronger skill preview and source badges
- [ ] Add finer-grained permissions for custom tools
- [ ] Add `run_command` isolation / sandbox support

## UX improvements

- [ ] Show richer skill usage summaries in conversation
- [ ] Add direct scope switching from UI or slash command
- [ ] Add setup/status hint for current runtime scope
- [ ] Add `.mybot/.env.example`

## Reliability

- [ ] Add regression coverage for resize / cursor edge cases
- [ ] Add validation for malformed custom tool manifests
- [ ] Add better diagnostics when config or `.env` is missing

## Future ideas

- [ ] Git-backed undo/redo
- [ ] Plan/apply agent mode
- [ ] Skill picker search/filter
- [ ] Tool usage history / audit view
