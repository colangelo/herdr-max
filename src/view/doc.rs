//! The far side's `herdr-app` document as the view reads it (core-phase5 D6,
//! D11). It is untrusted input: parsed here, once, with size, depth, count
//! and id-format limits; unknown fields are ignored; a document that breaks
//! a limit or has a wrong-typed field herdr reads is refused whole, so the
//! view keeps its last good layout. Only spaces marked `"herdr_max": 1`
//! (the far herdr's own, core-spaces-doc D6) are read; other clients' spaces
//! are not looked into.

use serde_json::Value;

/// Core's own limit on one document (`doc.set`).
pub(crate) const MAX_DOC_BYTES: usize = 1024 * 1024;
/// Nesting of layout nodes in one tab; herdr's own splits stay far below.
pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_SPACES: usize = 64;
pub(crate) const MAX_TABS: usize = 64;
/// Panes in one tab, and in the whole view.
pub(crate) const MAX_TAB_PANES: usize = 64;
pub(crate) const MAX_PANES: usize = 256;
/// Characters of a space or tab name, and of a folder.
pub(crate) const MAX_NAME_CHARS: usize = 256;
pub(crate) const MAX_FOLDER_CHARS: usize = 4096;
/// Longest id herdr writes: `w<n>:p<n>` with a few base-32 digits each.
const MAX_ID_BYTES: usize = 40;

/// What the view shows: the far herdr's spaces, tabs and splits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewLayout {
    pub spaces: Vec<ViewSpace>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewSpace {
    /// The far workspace id (`w3`), the view's id for it too.
    pub id: String,
    pub name: String,
    pub folder: String,
    pub tabs: Vec<ViewTab>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewTab {
    /// The far tab id (`w3:t2`).
    pub id: String,
    /// Its public number (`2` in `w3:t2`).
    pub number: usize,
    pub name: String,
    /// `None` for a tab with no pane in core (a PTY-only tab there).
    pub root: Option<ViewNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ViewNode {
    Pane {
        /// The far public pane id (`w3:p4`), the view's id for it.
        id: String,
        /// Its public number, `None` for a pane the far herdr has none for.
        number: Option<usize>,
        /// The far core's address of its terminal (`t7`); the view reaches it
        /// as `<machine>/t7`.
        terminal: String,
    },
    Split {
        /// Side by side (a `row`) or stacked (a `column`).
        side_by_side: bool,
        /// The first side's share, in (0, 1).
        ratio: f32,
        first: Box<ViewNode>,
        second: Box<ViewNode>,
    },
}

#[cfg(test)]
impl ViewLayout {
    /// Every pane's far terminal address, in order.
    pub(crate) fn terminals(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for space in &self.spaces {
            for tab in &space.tabs {
                if let Some(root) = &tab.root {
                    root.terminals(&mut out);
                }
            }
        }
        out
    }
}

#[cfg(test)]
impl ViewNode {
    fn terminals<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            ViewNode::Pane { terminal, .. } => out.push(terminal),
            ViewNode::Split { first, second, .. } => {
                first.terminals(out);
                second.terminals(out);
            }
        }
    }
}

/// Why a document was refused; the view logs it once and keeps its layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocError(pub String);

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse(why: impl Into<String>) -> DocError {
    DocError(why.into())
}

/// Reads the document core holds (`doc.get`'s `value`). `null` (never
/// written, or removed) is an empty layout.
pub(crate) fn parse(value: &Value) -> Result<ViewLayout, DocError> {
    let size = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_DOC_BYTES {
        return Err(refuse(format!("the document is {size} bytes, over 1 MiB")));
    }
    let fields = match value {
        Value::Null => return Ok(ViewLayout { spaces: Vec::new() }),
        Value::Object(fields) => fields,
        _ => return Err(refuse("the document is not an object")),
    };
    let spaces = match fields.get("spaces") {
        None | Some(Value::Null) => return Ok(ViewLayout { spaces: Vec::new() }),
        Some(Value::Array(spaces)) => spaces,
        Some(_) => return Err(refuse("spaces is not a list")),
    };
    let ours: Vec<&serde_json::Map<String, Value>> = spaces
        .iter()
        .filter_map(Value::as_object)
        .filter(|space| space.get("herdr_max").and_then(Value::as_u64) == Some(1))
        .collect();
    if ours.len() > MAX_SPACES {
        return Err(refuse(format!(
            "{} herdr-max spaces, over {MAX_SPACES}",
            ours.len()
        )));
    }
    let mut budget = MAX_PANES;
    let spaces = ours
        .into_iter()
        .map(|space| parse_space(space, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = std::collections::HashSet::new();
    for space in &spaces {
        if !seen.insert(space.id.as_str()) {
            return Err(refuse(format!("space {} is listed twice", space.id)));
        }
    }
    Ok(ViewLayout { spaces })
}

fn string_field<'a>(
    fields: &'a serde_json::Map<String, Value>,
    key: &str,
    what: &str,
) -> Result<&'a str, DocError> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(text)) => Ok(text),
        Some(_) => Err(refuse(format!("{what} {key} is not a string"))),
    }
}

fn limited(text: &str, max: usize, what: &str) -> Result<String, DocError> {
    if text.chars().count() > max {
        return Err(refuse(format!("{what} is longer than {max} characters")));
    }
    // Control characters would draw on the far side's behalf in our chrome.
    Ok(text.chars().filter(|ch| !ch.is_control()).collect())
}

fn parse_space(
    fields: &serde_json::Map<String, Value>,
    budget: &mut usize,
) -> Result<ViewSpace, DocError> {
    let id = string_field(fields, "id", "a space's")?;
    if !is_workspace_id(id) {
        return Err(refuse(format!(
            "space id {id:?} is not a herdr workspace id"
        )));
    }
    let name = limited(
        string_field(fields, "name", "space")?,
        MAX_NAME_CHARS,
        "a space name",
    )?;
    let folder = limited(
        string_field(fields, "folder", "space")?,
        MAX_FOLDER_CHARS,
        "a space folder",
    )?;
    let tabs = match fields.get("tabs") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(tabs)) => tabs.as_slice(),
        Some(_) => return Err(refuse(format!("space {id}: tabs is not a list"))),
    };
    if tabs.len() > MAX_TABS {
        return Err(refuse(format!(
            "space {id} has {} tabs, over {MAX_TABS}",
            tabs.len()
        )));
    }
    let tabs = tabs
        .iter()
        .map(|tab| match tab {
            Value::Object(tab) => parse_tab(id, tab, budget),
            _ => Err(refuse(format!("space {id}: a tab is not an object"))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ViewSpace {
        id: id.to_string(),
        name,
        folder,
        tabs,
    })
}

fn parse_tab(
    space: &str,
    fields: &serde_json::Map<String, Value>,
    budget: &mut usize,
) -> Result<ViewTab, DocError> {
    let id = string_field(fields, "id", "a tab's")?;
    let number = id
        .strip_prefix(space)
        .and_then(|rest| rest.strip_prefix(":t"))
        .and_then(public_number)
        .ok_or_else(|| refuse(format!("tab id {id:?} is not a tab of {space}")))?;
    let name = limited(
        string_field(fields, "name", "tab")?,
        MAX_NAME_CHARS,
        "a tab name",
    )?;
    let root = match fields.get("root") {
        None | Some(Value::Null) => None,
        Some(Value::Object(root)) => {
            let mut panes = 0;
            parse_node(space, root, 1, &mut panes)?
        }
        Some(_) => return Err(refuse(format!("tab {id}: root is not an object"))),
    };
    let count = root.as_ref().map_or(0, count_panes);
    if count > MAX_TAB_PANES {
        return Err(refuse(format!(
            "tab {id} has {count} panes, over {MAX_TAB_PANES}"
        )));
    }
    *budget = budget
        .checked_sub(count)
        .ok_or_else(|| refuse(format!("the document has more than {MAX_PANES} panes")))?;
    Ok(ViewTab {
        id: id.to_string(),
        number,
        name,
        root,
    })
}

fn count_panes(node: &ViewNode) -> usize {
    match node {
        ViewNode::Pane { .. } => 1,
        ViewNode::Split { first, second, .. } => count_panes(first) + count_panes(second),
    }
}

/// One layout node; `None` for a subtree with no pane in core (a container
/// left empty, or a pane whose terminal is on another machine of the far
/// core). `panes` counts the blocks seen, to stop a wide tree early.
fn parse_node(
    space: &str,
    fields: &serde_json::Map<String, Value>,
    depth: usize,
    panes: &mut usize,
) -> Result<Option<ViewNode>, DocError> {
    if depth > MAX_DEPTH {
        return Err(refuse(format!(
            "a layout is nested deeper than {MAX_DEPTH}"
        )));
    }
    let kind = match fields.get("type") {
        Some(Value::String(kind)) => kind.as_str(),
        _ => return Err(refuse("a layout node has no type")),
    };
    match kind {
        "block" => {
            *panes += 1;
            if *panes > MAX_TAB_PANES {
                return Err(refuse(format!("a tab has more than {MAX_TAB_PANES} panes")));
            }
            parse_block(space, fields)
        }
        "row" | "column" => {
            let children = match fields.get("children") {
                None | Some(Value::Null) => &[][..],
                Some(Value::Array(children)) => children.as_slice(),
                Some(_) => return Err(refuse("a container's children is not a list")),
            };
            if children.len() > MAX_TAB_PANES {
                return Err(refuse(format!(
                    "a container has more than {MAX_TAB_PANES} children"
                )));
            }
            let mut sides = Vec::with_capacity(children.len());
            for child in children {
                let Value::Object(child) = child else {
                    return Err(refuse("a layout child is not an object"));
                };
                let weight = weight(child)?;
                if let Some(node) = parse_node(space, child, depth + 1, panes)? {
                    sides.push((node, weight));
                }
            }
            Ok(fold_sides(sides, kind == "row"))
        }
        // A kind this build does not know (an app's floating pane, a mod):
        // not a terminal herdr can show.
        _ => Ok(None),
    }
}

fn parse_block(
    space: &str,
    fields: &serde_json::Map<String, Value>,
) -> Result<Option<ViewNode>, DocError> {
    let id = string_field(fields, "id", "a pane's")?;
    let Some(content) = fields.get("content") else {
        return Ok(None);
    };
    let Value::Object(content) = content else {
        return Err(refuse(format!("pane {id}: content is not an object")));
    };
    let terminal = match content.get("terminal") {
        // A mod or another app's content: nothing to attach.
        None => return Ok(None),
        Some(Value::String(terminal)) => terminal.as_str(),
        Some(_) => return Err(refuse(format!("pane {id}: terminal is not a string"))),
    };
    // A far pane on a machine of the far core (`m4m/t3`) is out of this
    // core's reach; only the far core's own terminals are shown.
    if !is_local_terminal(terminal) {
        return Ok(None);
    }
    let rest = id
        .strip_prefix(space)
        .ok_or_else(|| refuse(format!("pane id {id:?} is not a pane of {space}")))?;
    let number = if let Some(number) = rest.strip_prefix(":p") {
        Some(public_number(number).ok_or_else(|| refuse(format!("pane id {id:?}")))?)
    } else if rest.strip_prefix(":x").is_some_and(|raw| {
        !raw.is_empty() && raw.len() <= 10 && raw.bytes().all(|b| b.is_ascii_digit())
    }) {
        None
    } else {
        return Err(refuse(format!("pane id {id:?} is not a herdr pane id")));
    };
    Ok(Some(ViewNode::Pane {
        id: id.to_string(),
        number,
        terminal: terminal.to_string(),
    }))
}

/// A child's share in its container: herdr writes `{"weight": w}`; a size
/// in pixels, or none, counts as an equal share.
fn weight(fields: &serde_json::Map<String, Value>) -> Result<f32, DocError> {
    match fields.get("size") {
        None | Some(Value::Null) => Ok(1.0),
        Some(Value::Object(size)) => match size.get("weight") {
            Some(Value::Number(weight)) => weight
                .as_f64()
                .filter(|weight| weight.is_finite() && *weight > 0.0 && *weight <= 1.0e6)
                .map(|weight| weight as f32)
                .ok_or_else(|| refuse("a weight is not a positive number")),
            Some(_) => Err(refuse("a weight is not a number")),
            None => Ok(1.0),
        },
        Some(_) => Err(refuse("a size is not an object")),
    }
}

/// A container's sides as binary splits, the first side against the rest,
/// each split's ratio that side's share of what is left.
fn fold_sides(mut sides: Vec<(ViewNode, f32)>, side_by_side: bool) -> Option<ViewNode> {
    let (last, last_weight) = sides.pop()?;
    let mut node = last;
    let mut rest_weight = last_weight;
    while let Some((side, weight)) = sides.pop() {
        let ratio = crate::layout::valid_split_ratio(weight / (weight + rest_weight));
        node = ViewNode::Split {
            side_by_side,
            ratio,
            first: Box::new(side),
            second: Box::new(node),
        };
        rest_weight += weight;
    }
    Some(node)
}

/// `w` and a herdr public number, as `generate_workspace_id` writes.
fn is_workspace_id(id: &str) -> bool {
    id.len() <= MAX_ID_BYTES && id.strip_prefix('w').and_then(public_number).is_some()
}

fn public_number(raw: &str) -> Option<usize> {
    if raw.is_empty() || raw.len() > 12 {
        return None;
    }
    crate::workspace::decode_public_number(raw).filter(|number| *number > 0)
}

/// A terminal of the far core itself: `t` and digits, as core numbers them.
fn is_local_terminal(address: &str) -> bool {
    address.strip_prefix('t').is_some_and(|digits| {
        !digits.is_empty() && digits.len() <= 9 && digits.bytes().all(|b| b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn leaf(id: &str, terminal: &str, weight: Option<f32>) -> Value {
        let mut node = json!({"id": id, "type": "block", "content": {"terminal": terminal}});
        if let Some(weight) = weight {
            node["size"] = json!({"weight": weight});
        }
        node
    }

    fn doc(tabs: Value) -> Value {
        json!({"spaces": [
            {"id": "w9", "name": "other app", "tabs": "not ours, not read"},
            {"id": "w3", "name": "api", "folder": "/Users/ac/api", "herdr_max": 1, "color": "teal", "tabs": tabs}
        ], "extra": true})
    }

    fn one_tab(root: Value) -> Value {
        doc(json!([{"id": "w3:t1", "name": "", "root": root}]))
    }

    #[test]
    fn herdr_spaces_tabs_and_splits_are_read_and_other_spaces_are_not() {
        let value = doc(json!([
            {"id": "w3:t1", "name": "", "root": {"id": "w3:t1", "type": "row", "children": [
                leaf("w3:p1", "t1", Some(0.7)),
                {"id": "w3:t1:sb", "type": "column", "size": {"weight": 0.3}, "children": [
                    leaf("w3:p2", "t2", Some(0.5)), leaf("w3:p3", "t5", Some(0.5))
                ]}
            ]}, "unknown": [1, 2]},
            {"id": "w3:t2", "name": "logs", "root": {"id": "w3:t2", "type": "row", "children": [leaf("w3:p4", "t9", None)]}}
        ]));
        let layout = parse(&value).expect("parses");
        assert_eq!(layout.spaces.len(), 1, "only the herdr_max space");
        let space = &layout.spaces[0];
        assert_eq!(
            (
                space.id.as_str(),
                space.name.as_str(),
                space.folder.as_str()
            ),
            ("w3", "api", "/Users/ac/api")
        );
        assert_eq!(space.tabs[0].number, 1);
        assert_eq!(space.tabs[1].name, "logs");
        let Some(ViewNode::Split {
            side_by_side,
            ratio,
            first,
            second,
        }) = &space.tabs[0].root
        else {
            panic!("a split")
        };
        assert!(*side_by_side);
        assert!((ratio - 0.7).abs() < 1e-6);
        assert_eq!(
            **first,
            ViewNode::Pane {
                id: "w3:p1".into(),
                number: Some(1),
                terminal: "t1".into()
            }
        );
        assert!(matches!(
            **second,
            ViewNode::Split {
                side_by_side: false,
                ..
            }
        ));
        assert_eq!(layout.terminals(), ["t1", "t2", "t5", "t9"]);
        assert_eq!(
            space.tabs[1].root,
            Some(ViewNode::Pane {
                id: "w3:p4".into(),
                number: Some(4),
                terminal: "t9".into()
            }),
            "a row of one is its pane"
        );
    }

    #[test]
    fn three_sides_fold_into_two_splits_with_their_shares() {
        let layout = parse(&one_tab(json!({"id": "w3:t1", "type": "row", "children": [
            leaf("w3:p1", "t1", Some(1.0)), leaf("w3:p2", "t2", Some(1.0)), leaf("w3:p3", "t3", Some(2.0))
        ]})))
        .unwrap();
        let Some(ViewNode::Split { ratio, second, .. }) = &layout.spaces[0].tabs[0].root else {
            panic!()
        };
        assert!((ratio - 0.25).abs() < 1e-6);
        let ViewNode::Split { ratio, .. } = &**second else {
            panic!()
        };
        assert!((ratio - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn null_and_empty_documents_are_an_empty_layout() {
        assert!(parse(&Value::Null).unwrap().spaces.is_empty());
        assert!(parse(&json!({})).unwrap().spaces.is_empty());
        assert!(parse(&json!({"spaces": []})).unwrap().spaces.is_empty());
    }

    // V16 (D11): a bad document is refused whole, so the view keeps its last
    // good layout; unknown fields are ignored (above).
    #[test]
    fn oversized_deep_and_wrong_typed_documents_are_refused() {
        let big = "x".repeat(MAX_DOC_BYTES);
        assert!(
            parse(&json!({"spaces": [], "pad": big})).is_err(),
            "over 1 MiB"
        );

        let mut deep = leaf("w3:p1", "t1", None);
        for _ in 0..MAX_DEPTH + 1 {
            deep = json!({"id": "w3:t1", "type": "row", "children": [deep]});
        }
        assert!(parse(&one_tab(deep)).is_err(), "too deep");

        for bad in [
            json!(["a list"]),
            json!({"spaces": {"id": "w3"}}),
            doc(json!("tabs as a string")),
            doc(json!([{"id": "w3:t1", "name": 7}])),
            doc(json!([{"id": "w3:t1", "root": "a string"}])),
            one_tab(
                json!({"id": "w3:t1", "type": "row", "children": [{"id": "w3:p1", "type": "block", "content": {"terminal": 7}}]}),
            ),
            one_tab(
                json!({"id": "w3:t1", "type": "row", "children": [leaf("w3:p1", "t1", Some(-1.0))]}),
            ),
            one_tab(json!({"id": "w3:t1", "type": "row", "children": "x"})),
        ] {
            assert!(parse(&bad).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn ids_must_be_herdr_ids_of_their_space() {
        let space = |id: &str| json!({"spaces": [{"id": id, "herdr_max": 1, "tabs": []}]});
        assert!(parse(&space("w3")).is_ok());
        for bad in [
            "",
            "x3",
            "w",
            "w3/../x",
            "w3;rm",
            &format!("w{}", "1".repeat(40)),
        ] {
            assert!(parse(&space(bad)).is_err(), "space id {bad:?}");
        }
        assert!(
            parse(&doc(json!([{"id": "w4:t1"}]))).is_err(),
            "a tab of another space"
        );
        assert!(parse(&doc(json!([{"id": "w3:tx!"}]))).is_err());
        assert!(parse(&one_tab(
            json!({"id": "w3:t1", "type": "row", "children": [leaf("w4:p1", "t1", None)]})
        ))
        .is_err());
        assert!(parse(&one_tab(
            json!({"id": "w3:t1", "type": "row", "children": [leaf("w3:q1", "t1", None)]})
        ))
        .is_err());
        let raw = parse(&one_tab(
            json!({"id": "w3:t1", "type": "row", "children": [leaf("w3:x17", "t1", None)]}),
        ))
        .unwrap();
        assert!(matches!(
            raw.spaces[0].tabs[0].root,
            Some(ViewNode::Pane { number: None, .. })
        ));
        let twice = json!({"spaces": [{"id": "w3", "herdr_max": 1}, {"id": "w3", "herdr_max": 1}]});
        assert!(parse(&twice).is_err());
    }

    #[test]
    fn panes_out_of_reach_are_left_out_and_counts_are_bounded() {
        let layout = parse(&one_tab(json!({"id": "w3:t1", "type": "row", "children": [
            leaf("w3:p1", "m4m/t3", None),
            leaf("w3:p2", "t2", None),
            {"id": "w3:p3", "type": "block", "content": {"mod": "clock"}},
            {"id": "w3:p4", "type": "float"}
        ]})))
        .unwrap();
        assert_eq!(
            layout.terminals(),
            ["t2"],
            "a far machine pane, a mod and an unknown kind are left out"
        );

        let wide: Vec<Value> = (1..=MAX_TAB_PANES + 1)
            .map(|n| {
                leaf(
                    &format!("w3:p{}", crate::workspace::encode_public_number(n)),
                    &format!("t{n}"),
                    None,
                )
            })
            .collect();
        assert!(parse(&one_tab(
            json!({"id": "w3:t1", "type": "row", "children": wide})
        ))
        .is_err());

        let names = json!({"spaces": [{"id": "w3", "herdr_max": 1, "name": "y".repeat(MAX_NAME_CHARS + 1)}]});
        assert!(parse(&names).is_err());
        let control =
            parse(&json!({"spaces": [{"id": "w3", "herdr_max": 1, "name": "a\u{1b}[2Jb"}]}))
                .unwrap();
        assert_eq!(
            control.spaces[0].name, "a[2Jb",
            "control characters are dropped"
        );
    }
}
