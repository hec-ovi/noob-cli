# monitor

contractVersion: 1.0.0

## Purpose

What the machine and this run are costing, as three gauge lists: HARDWARE
(is the machine keeping up, from /sys and /proc), CONTEXT (what this run
holds now), SESSION (what it has spent altogether).

## Public surface

```rust
pub struct Gauge;            // label, reading text, optional fraction
impl Gauge {
    pub fn fraction(&self) -> Option<f32>;   // for a bar, when bounded
    pub fn reading(&self) -> String;
}
pub struct Monitor;
impl Monitor {
    pub fn new() -> Monitor;
    pub fn hardware(&self) -> Vec<Gauge>;
    pub fn context(&self) -> Vec<Gauge>;
    pub fn session(&self) -> Vec<Gauge>;
    pub fn sample(&mut self, state: &State);   // refresh from the reducer
}
```

## Invariants

1. Hardware gauges degrade to fewer rows off-Linux by design: a missing
   /proc reading drops a gauge, never errors.
2. Context and session read only the reducer's state; every number shown
   is the window that is open.
3. Sampling is pull: nothing here ticks on its own.

## Dependencies

Contracts: the reducer's state (crate::state, until it boxes), the style
box for nothing: gauges are data, the view paints them.

## Tests

Inline: reading formats, fractions, degradation (13 tests).
