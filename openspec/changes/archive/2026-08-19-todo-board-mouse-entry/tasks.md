## 1. The count

- [x] 1.1 A session-wide outstanding-todos read on `AppState`: count plus highest priority, one pass, no allocation

## 2. The tab bar

- [x] 2.1 `todo_indicator_label` / width beside the notification ones: bare glyph at zero, glyph + count otherwise, 99+ cap like notifications
- [x] 2.2 Lay the todo indicator out immediately left of the notification indicator, and thread `todo_hit_area` through `TabBarView` into `ViewState`
- [x] 2.3 Color it by the highest outstanding priority, the border indicator's rule
- [x] 2.4 Swap the notification glyph `◆` → `и` in the indicator label

## 3. The click

- [x] 3.1 A click on the hit area toggles the board through the same paths as the keybinding

## 4. Tests

- [x] 4.1 Label unit tests: bare, counted, 99+ — both indicators
- [x] 4.2 Layout: the two indicators sit in the trailing corner, todo left of notification, hit areas disjoint
- [x] 4.3 Click toggles the board open and closed
- [x] 4.4 The count and color aggregate across spaces
- [x] 4.5 Update the notification tests asserting `◆`

## 5. Verification

- [x] 5.1 `just check` green
- [x] 5.2 Dogfood on the `-ac-beta` channel
