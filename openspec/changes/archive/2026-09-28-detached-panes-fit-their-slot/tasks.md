## 1. Detached sizing (issue 95)

- [x] 1.1 `detached_pane_size` set by the headless server; `estimate_pane_size` uses it; tests
- [x] 1.2 Splits divide the target's size while detached (`split_shares`, `split_sizes`); tests

## 2. Ship

- [x] 2.1 `just check` green
- [x] 2.2 Beta; throwaway proof: a split created with no client attached divides the target

Verified 2026-09-28 on m4m with the `0.8.2-ac-beta.102-mckennie` release binary: with no client attached, a down split of a new workspace gave PTYs 26×120 and 14×120, where beta.101 gave both 39×47 (the displayed pane's size).
