//! tmux layout-string parser (DESIGN.md §4.2): `%layout-change` carries e.g.
//! `bd5b,159x48,0,0{79x48,0,0,1,79x48,80,0[79x24,80,0,2,79x23,80,25,3]}`
//! — `checksum,WxH,X,Y` nodes, nested with `{}` (left/right splits) and
//! `[]` (top/bottom splits), leaves ending in `,<pane-number>`.

use super::PaneId;
use crate::error::EngineError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRect {
    pub width: u16,
    pub height: u16,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNode {
    /// A pane. `pane` is the `%N` id (layout strings carry the number).
    Leaf { rect: LayoutRect, pane: PaneId },
    /// Left-right arrangement (`{…}`).
    Horizontal {
        rect: LayoutRect,
        children: Vec<LayoutNode>,
    },
    /// Top-bottom arrangement (`[…]`).
    Vertical {
        rect: LayoutRect,
        children: Vec<LayoutNode>,
    },
}

impl LayoutNode {
    pub fn rect(&self) -> &LayoutRect {
        match self {
            LayoutNode::Leaf { rect, .. }
            | LayoutNode::Horizontal { rect, .. }
            | LayoutNode::Vertical { rect, .. } => rect,
        }
    }

    /// All panes with their geometry, in layout order.
    pub fn leaves(&self) -> Vec<(&PaneId, &LayoutRect)> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<(&'a PaneId, &'a LayoutRect)>) {
        match self {
            LayoutNode::Leaf { rect, pane } => out.push((pane, rect)),
            LayoutNode::Horizontal { children, .. } | LayoutNode::Vertical { children, .. } => {
                for c in children {
                    c.collect(out);
                }
            }
        }
    }
}

pub fn parse_layout(s: &str) -> Result<LayoutNode, EngineError> {
    let err = |what: &str| EngineError::Parse(format!("layout {what}: {s:?}"));

    // Strip "checksum," prefix (4 hex digits).
    let body = match s.split_once(',') {
        Some((csum, rest)) if csum.len() == 4 && csum.chars().all(|c| c.is_ascii_hexdigit()) => {
            rest
        }
        _ => return Err(err("missing checksum")),
    };

    let bytes = body.as_bytes();
    let mut pos = 0;
    let node = parse_node(bytes, &mut pos).ok_or_else(|| err("malformed"))?;
    if pos != bytes.len() {
        return Err(err("trailing garbage"));
    }
    Ok(node)
}

fn parse_u16(bytes: &[u8], pos: &mut usize) -> Option<u16> {
    let start = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if start == *pos {
        return None;
    }
    std::str::from_utf8(&bytes[start..*pos]).ok()?.parse().ok()
}

fn expect(bytes: &[u8], pos: &mut usize, b: u8) -> Option<()> {
    if bytes.get(*pos) == Some(&b) {
        *pos += 1;
        Some(())
    } else {
        None
    }
}

fn parse_node(bytes: &[u8], pos: &mut usize) -> Option<LayoutNode> {
    let width = parse_u16(bytes, pos)?;
    expect(bytes, pos, b'x')?;
    let height = parse_u16(bytes, pos)?;
    expect(bytes, pos, b',')?;
    let x = parse_u16(bytes, pos)?;
    expect(bytes, pos, b',')?;
    let y = parse_u16(bytes, pos)?;
    let rect = LayoutRect {
        width,
        height,
        x,
        y,
    };

    match bytes.get(*pos) {
        Some(b',') => {
            // Leaf: ",<pane-number>"
            *pos += 1;
            let num = parse_u16(bytes, pos)?;
            Some(LayoutNode::Leaf {
                rect,
                pane: PaneId(format!("%{num}")),
            })
        }
        Some(&open @ (b'{' | b'[')) => {
            *pos += 1;
            let close = if open == b'{' { b'}' } else { b']' };
            let mut children = Vec::new();
            loop {
                children.push(parse_node(bytes, pos)?);
                match bytes.get(*pos) {
                    Some(b',') => {
                        *pos += 1;
                    }
                    Some(&c) if c == close => {
                        *pos += 1;
                        break;
                    }
                    _ => return None,
                }
            }
            if children.is_empty() {
                return None;
            }
            Some(if open == b'{' {
                LayoutNode::Horizontal { rect, children }
            } else {
                LayoutNode::Vertical { rect, children }
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_pane() {
        let n = parse_layout("bd5b,80x24,0,0,3").unwrap();
        match &n {
            LayoutNode::Leaf { rect, pane } => {
                assert_eq!(pane.as_str(), "%3");
                assert_eq!((rect.width, rect.height, rect.x, rect.y), (80, 24, 0, 0));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(n.leaves().len(), 1);
    }

    #[test]
    fn nested_splits() {
        let s = "b25f,159x48,0,0{79x48,0,0,1,79x48,80,0[79x24,80,0,2,79x23,80,25,3]}";
        let n = parse_layout(s).unwrap();
        let leaves = n.leaves();
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[0].0.as_str(), "%1");
        assert_eq!(leaves[1].0.as_str(), "%2");
        assert_eq!(leaves[2].0.as_str(), "%3");
        let (_, r3) = leaves[2];
        assert_eq!((r3.width, r3.height, r3.x, r3.y), (79, 23, 80, 25));
        match n {
            LayoutNode::Horizontal { ref children, .. } => {
                assert!(matches!(children[1], LayoutNode::Vertical { .. }));
            }
            ref other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse_layout("nochecksum").is_err());
        assert!(parse_layout("bd5b,80x24,0,0").is_err()); // no pane id
        assert!(parse_layout("bd5b,80x24,0,0{79x24,0,0,1").is_err()); // unclosed
        assert!(parse_layout("zzzz,80x24,0,0,3xtrail").is_err());
    }
}
