# Control Group Extraction Pattern

Use control group extraction when UI controls for one product area are spread across several infrastructure files and every new control requires touching all of them.

The basic idea is to move one coherent set of controls into a small adapter-local module that owns the repeated control facts:

- control IDs used by the group
- labels and parent containers
- initial styles
- event-to-message routing
- dynamic render updates such as enabled, checked, or selected state

The group should not own unrelated business rules, reducers, generic UI framework behavior, or broad layout policy. It is a packaging pattern for UI adapter code, not a new UI framework.

## Good Fit

This pattern works well when controls are cohesive and change together. Examples:

- footer action buttons
- toolbar actions
- section expand/collapse toggles
- tab-specific action rows
- mode or stage radio groups
- dialog buttons with shared enablement rules

It is especially useful when adding one visible control currently requires coordinated edits in files like:

- constants
- startup/control creation
- theme application
- layout rules
- event handling
- render-state caching
- render command emission
- UI regression tests

After extraction, most future changes should happen in one group module plus focused tests.

## Poor Fit

Avoid this pattern when the proposed group is not actually cohesive.

Poor candidates include:

- all controls in a complex screen at once
- a mix of unrelated inputs, labels, actions, and layout rules
- controls whose routing depends heavily on live application state
- generic cross-product widgets that belong in UI infrastructure
- business logic that belongs in reducers, services, or domain modules

If the group becomes a dumping ground for an entire screen, it will reduce clarity instead of reducing blast radius.

## Recommended Shape

Create a module named after the product area and control group, for example:

```text
ui/groups/prompt_lab_actions.rs
ui/groups/footer_buttons.rs
ui/groups/search_filters.rs
```

The module usually exposes functions like:

```rust
create_controls(...)
apply_theme(...)
msg_for_button(...)
msg_for_checkbox(...)
render(...)
```

Use plain descriptors for static facts:

```rust
struct ButtonDescriptor {
    control_id: ControlId,
    parent_control_id: ControlId,
    label: &'static str,
    initial_style: StyleId,
    msg: Option<fn() -> Msg>,
}
```

Keep dynamic behavior explicit in `render(...)`. Descriptors should say what a control is, not hide changing business conditions.

## Process

1. Pick a narrow, cohesive control set.
2. Run baseline tests before moving code.
3. Add descriptors and descriptor tests.
4. Move control creation.
5. Move initial style application.
6. Move event routing for only descriptor-known controls.
7. Move dynamic render-state fields and render updates.
8. Leave unrelated controls in place.
9. Run focused tests after each phase, then full checks.

Do not use broad catch-all event routing. Route only controls known to the group, so unrelated controls keep their existing behavior.

## Expected Result

The visible UI and reducer behavior should not change. The benefit is organizational:

- fewer files touched when adding or changing a control
- one source of truth for static control metadata
- smaller regression surface
- clearer ownership boundaries between product UI code and generic UI infrastructure

The pattern is successful when a future control addition is mostly a descriptor entry, a render rule if needed, and a focused test.
