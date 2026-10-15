//! The difference between the world at two points of a history: a plain
//! comparison of two manifests as JSON.
//!
//! No format semantics live here. Which fields a world has and what they
//! mean is the reference crate's business — both sides come from its
//! fold (`WorldDoc::to_manifest`, serialized) — and this module only
//! says which values differ, reading them the way the spec's own
//! equality does (spec/package.md, "Head-first"): entities by id, an
//! absent field the same as `null`, numbers by value.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

/// Longest rendering of one value, in characters; longer ones end in `…`.
pub const MAX_RENDER: usize = 80;

/// How an entity, or a world field, differs between the two sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// On the `to` side only.
    Added,
    /// On the `from` side only.
    Removed,
    /// On both sides, with differing values.
    Changed,
}

impl Change {
    /// The word the inspector prints.
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Changed => "changed",
        }
    }
}

/// One value that differs: a top-level field, or — when both sides hold
/// an object — the sub-key inside it that does (`transform.position`).
/// Arrays compare whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDiff {
    /// The field's dotted path.
    pub path: String,
    /// The `from` side, rendered short; `None` when absent or `null`.
    pub from: Option<String>,
    /// The `to` side, rendered short; `None` when absent or `null`.
    pub to: Option<String>,
}

impl FieldDiff {
    /// Which way the field changed.
    pub fn change(&self) -> Change {
        match (&self.from, &self.to) {
            (None, Some(_)) => Change::Added,
            (Some(_), None) => Change::Removed,
            _ => Change::Changed,
        }
    }

    /// `from → to`, with `—` for an absent side.
    pub fn values(&self) -> String {
        format!(
            "{} → {}",
            self.from.as_deref().unwrap_or("—"),
            self.to.as_deref().unwrap_or("—")
        )
    }

    /// `path: from → to`.
    pub fn line(&self) -> String {
        format!("{}: {}", self.path, self.values())
    }
}

/// One entity that differs between the two sides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDiff {
    /// The entity's `id`, as JSON renders it (`#n`, its position, when
    /// an entity has none).
    pub id: String,
    /// The entity's `name` — the `to` side's when both have one.
    pub name: String,
    /// Added, removed, or changed.
    pub change: Change,
    /// The fields that differ, `id` excluded, in path order: every field
    /// of an added or removed entity, the changed ones of a changed one.
    pub fields: Vec<FieldDiff>,
}

/// Everything that differs between two worlds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldDiff {
    /// Entities that differ, in id order (numeric ids first, by value).
    pub entities: Vec<EntityDiff>,
    /// World-level fields that differ — everything but `entities` — in
    /// path order.
    pub world: Vec<FieldDiff>,
}

impl WorldDiff {
    /// Nothing differs.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.world.is_empty()
    }

    /// How many entities changed this way.
    pub fn count(&self, change: Change) -> usize {
        self.entities.iter().filter(|e| e.change == change).count()
    }
}

/// Compare two worlds, both manifests as JSON. Entities match by `id`;
/// every other top-level field compares as a world field. Neither value
/// needs to be an object — anything else counts as a world with nothing
/// in it.
pub fn diff_worlds(from: &Value, to: &Value) -> WorldDiff {
    let empty = Map::new();
    let from = from.as_object().unwrap_or(&empty);
    let to = to.as_object().unwrap_or(&empty);

    let from_entities = entities_by_id(from);
    let to_entities = entities_by_id(to);
    let mut keys: Vec<&str> = from_entities
        .keys()
        .chain(to_entities.keys())
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    keys.sort_by_key(|key| id_order(key));

    let mut entities = Vec::new();
    for key in keys {
        let (a, b) = (from_entities.get(key), to_entities.get(key));
        let change = match (a, b) {
            (None, Some(_)) => Change::Added,
            (Some(_), None) => Change::Removed,
            _ => Change::Changed,
        };
        let mut fields = Vec::new();
        diff_objects(
            "",
            a.copied().unwrap_or(&empty),
            b.copied().unwrap_or(&empty),
            &["id"],
            &mut fields,
        );
        if fields.is_empty() {
            continue;
        }
        let name = b
            .or(a)
            .and_then(|entity| entity.get("name"))
            .map(|name| match name {
                Value::String(name) => name.clone(),
                other => render(other),
            })
            .unwrap_or_default();
        entities.push(EntityDiff {
            id: key.to_string(),
            name,
            change,
            fields,
        });
    }

    let mut world = Vec::new();
    diff_objects("", from, to, &["entities"], &mut world);
    WorldDiff { entities, world }
}

/// The spec's equality: numbers by value, an absent member the same as
/// `null`, arrays element by element.
pub fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => number_eq(x, y),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| json_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => x
            .keys()
            .chain(y.keys())
            .all(|key| option_eq(present(x.get(key)), present(y.get(key)))),
        _ => a == b,
    }
}

/// One value, short: strings quoted, floats as the format wrote them,
/// arrays and objects inline, at most [`MAX_RENDER`] characters.
pub fn render(value: &Value) -> String {
    let mut out = String::new();
    render_into(value, &mut out);
    if let Some((cut, _)) = out.char_indices().nth(MAX_RENDER) {
        out.truncate(cut);
        out.push('…');
    }
    out
}

/// The `entities` array keyed by each entity's `id` (its position when
/// it has none). A missing or non-array `entities` is no entities;
/// an element that isn't an object is skipped.
fn entities_by_id(
    world: &Map<String, Value>,
) -> std::collections::BTreeMap<String, &Map<String, Value>> {
    world
        .get("entities")
        .and_then(Value::as_array)
        .map(|entities| {
            entities
                .iter()
                .enumerate()
                .filter_map(|(n, entity)| {
                    let entity = entity.as_object()?;
                    let id = match present(entity.get("id")) {
                        Some(id) => render(id),
                        None => format!("#{n}"),
                    };
                    Some((id, entity))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Numeric ids first, by value; the rest after, by text.
fn id_order(key: &str) -> (u8, u64, &str) {
    match key.parse::<u64>() {
        Ok(n) => (0, n, key),
        Err(_) => (1, 0, key),
    }
}

/// `null` is absent.
fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

fn option_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => json_eq(a, b),
        _ => false,
    }
}

fn number_eq(x: &Number, y: &Number) -> bool {
    if let (Some(p), Some(q)) = (x.as_i64(), y.as_i64()) {
        return p == q;
    }
    if let (Some(p), Some(q)) = (x.as_u64(), y.as_u64()) {
        return p == q;
    }
    match (x.as_f64(), y.as_f64()) {
        (Some(p), Some(q)) => p == q,
        _ => false,
    }
}

/// Every member of either object that differs, under `prefix`: both
/// sides objects recurse (`a.b.c`); anything else is one leaf. Keys in
/// `skip` are left out at this level only.
fn diff_objects(
    prefix: &str,
    from: &Map<String, Value>,
    to: &Map<String, Value>,
    skip: &[&str],
    out: &mut Vec<FieldDiff>,
) {
    let keys: BTreeSet<&str> = from
        .keys()
        .chain(to.keys())
        .map(String::as_str)
        .filter(|key| !skip.contains(key))
        .collect();
    for key in keys {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        let (a, b) = (present(from.get(key)), present(to.get(key)));
        match (a, b) {
            (None, None) => {}
            (Some(Value::Object(x)), Some(Value::Object(y))) => {
                diff_objects(&path, x, y, &[], out);
            }
            (a, b) => {
                if !option_eq(a, b) {
                    out.push(FieldDiff {
                        path,
                        from: a.map(render),
                        to: b.map(render),
                    });
                }
            }
        }
    }
}

fn render_into(value: &Value, out: &mut String) {
    // Past this many bytes the text holds more than MAX_RENDER characters
    // whatever it says, and `render` will cut it — stop building it.
    const ENOUGH: usize = 4 * MAX_RENDER;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => {
            let _ = write!(out, "{b}");
        }
        Value::Number(n) => out.push_str(&render_number(n)),
        Value::String(s) => {
            let _ = write!(out, "{s:?}");
        }
        Value::Array(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                if out.len() > ENOUGH {
                    break;
                }
                render_into(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            out.push('{');
            for (n, (key, item)) in members.iter().enumerate() {
                if n > 0 {
                    out.push_str(", ");
                }
                if out.len() > ENOUGH {
                    break;
                }
                out.push_str(key);
                out.push_str(": ");
                render_into(item, out);
            }
            out.push('}');
        }
    }
}

/// Integers as they are; a float that survives the trip through `f32`
/// prints as that `f32` — the format's numbers are 32-bit, and
/// serializing them to JSON widens `2.4` to `2.4000000953674316`.
fn render_number(n: &Number) -> String {
    if n.is_i64() || n.is_u64() {
        return n.to_string();
    }
    let Some(f) = n.as_f64() else {
        return n.to_string();
    };
    let narrow = f as f32;
    if f64::from(narrow) == f {
        format!("{narrow}")
    } else {
        format!("{f}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn world(entities: Value) -> Value {
        json!({
            "version": 3,
            "meta": { "name": "probe" },
            "camera": { "position": [0, 6, 16], "look_at": [0, 1, -2] },
            "entities": entities,
            "next_entity_id": 3
        })
    }

    #[test]
    fn identical_worlds_have_no_diff() {
        let a =
            world(json!([{ "id": 1, "name": "ground", "transform": { "position": [0, 0, 0] } }]));
        assert!(diff_worlds(&a, &a).is_empty());
    }

    #[test]
    fn absent_and_null_are_equal_and_numbers_compare_by_value() {
        let a = world(json!([{
            "id": 1, "name": "ground", "parent": null,
            "transform": { "position": [0, 1, 2], "rotation": null }
        }]));
        let b = world(json!([{
            "id": 1, "name": "ground",
            "transform": { "position": [0.0, 1.0, 2.0] }
        }]));
        assert!(diff_worlds(&a, &b).is_empty());
        assert!(json_eq(&json!(1), &json!(1.0)));
        assert!(json_eq(&json!({ "a": null }), &json!({})));
        assert!(!json_eq(&json!([1, 2]), &json!([1, 2, 3])));
        assert!(!json_eq(&json!("1"), &json!(1)));
    }

    #[test]
    fn spawned_and_deleted_entities_classify() {
        let a = world(json!([
            { "id": 1, "name": "ground", "transform": { "position": [0, 0, 0] } },
            { "id": 2, "name": "sun", "light": { "intensity": 12000 } }
        ]));
        let b = world(json!([
            { "id": 2, "name": "sun", "light": { "intensity": 12000 } },
            { "id": 3, "name": "lantern", "shape": { "Cylinder": { "radius": 0.12, "height": 0.9 } } }
        ]));
        let diff = diff_worlds(&a, &b);
        let rows: Vec<(&str, Change)> = diff
            .entities
            .iter()
            .map(|e| (e.id.as_str(), e.change))
            .collect();
        assert_eq!(rows, vec![("1", Change::Removed), ("3", Change::Added)]);
        assert_eq!(diff.count(Change::Removed), 1);
        assert_eq!(diff.count(Change::Added), 1);
        assert_eq!(diff.count(Change::Changed), 0);
        assert!(diff.world.is_empty());

        let removed = &diff.entities[0];
        assert_eq!(removed.name, "ground");
        assert_eq!(
            removed.fields,
            vec![
                FieldDiff {
                    path: "name".into(),
                    from: Some("\"ground\"".into()),
                    to: None
                },
                FieldDiff {
                    path: "transform".into(),
                    from: Some("{position: [0, 0, 0]}".into()),
                    to: None
                },
            ]
        );
        let added = &diff.entities[1];
        assert_eq!(added.name, "lantern");
        assert_eq!(
            added.fields.iter().map(|f| f.line()).collect::<Vec<_>>(),
            vec![
                "name: — → \"lantern\"",
                "shape: — → {Cylinder: {height: 0.9, radius: 0.12}}",
            ]
        );
    }

    #[test]
    fn a_changed_entity_lists_its_changed_subkeys() {
        let a = world(json!([{
            "id": 1, "name": "banner",
            "transform": { "position": [-1.5, 4.2, -7.2], "scale": [1, 1, 1] },
            "shape": { "Cuboid": { "x": 2.4, "y": 0.5, "z": 0.08 } }
        }]));
        let b = world(json!([{
            "id": 1, "name": "flag",
            "transform": { "position": [-1.5, 4.6, -7.2], "scale": [1, 1, 1] },
            "shape": { "Cuboid": { "x": 2.6, "y": 0.5, "z": 0.08 } },
            "material": { "color": [0.85, 0.8, 0.7, 1] }
        }]));
        let diff = diff_worlds(&a, &b);
        assert_eq!(diff.entities.len(), 1);
        let entity = &diff.entities[0];
        assert_eq!(entity.change, Change::Changed);
        assert_eq!(entity.name, "flag");
        assert_eq!(
            entity.fields.iter().map(|f| f.line()).collect::<Vec<_>>(),
            vec![
                "material: — → {color: [0.85, 0.8, 0.7, 1]}",
                "name: \"banner\" → \"flag\"",
                "shape.Cuboid.x: 2.4 → 2.6",
                "transform.position: [-1.5, 4.2, -7.2] → [-1.5, 4.6, -7.2]",
            ]
        );
        assert_eq!(entity.fields[0].change(), Change::Added);
        assert_eq!(entity.fields[1].change(), Change::Changed);
    }

    #[test]
    fn arrays_compare_whole() {
        let a = world(json!([{
            "id": 1, "name": "orb",
            "behaviors": [{ "Spin": { "axis": [0, 1, 0], "speed": 1 } }]
        }]));
        let b = world(json!([{
            "id": 1, "name": "orb",
            "behaviors": [{ "Spin": { "axis": [0, 1, 0], "speed": 2 } }, { "Bob": { "amplitude": 0.5 } }]
        }]));
        let diff = diff_worlds(&a, &b);
        assert_eq!(diff.entities[0].fields.len(), 1);
        assert_eq!(
            diff.entities[0].fields[0].line(),
            "behaviors: [{Spin: {axis: [0, 1, 0], speed: 1}}] → \
             [{Spin: {axis: [0, 1, 0], speed: 2}}, {Bob: {amplitude: 0.5}}]"
        );
    }

    #[test]
    fn world_fields_compare_outside_the_entities() {
        let entities = json!([{ "id": 1, "name": "ground" }]);
        let a = world(entities.clone());
        let mut b = world(entities);
        b["meta"]["description"] = json!("a probe world");
        b["camera"]["position"] = json!([0, 8, 16]);
        b["next_entity_id"] = json!(9);
        b["soundtrack"] = json!({ "path": "song.ogg", "bpm": 120 });
        b.as_object_mut().unwrap().remove("version");
        let diff = diff_worlds(&a, &b);
        assert!(diff.entities.is_empty());
        assert_eq!(
            diff.world.iter().map(|f| f.line()).collect::<Vec<_>>(),
            vec![
                "camera.position: [0, 6, 16] → [0, 8, 16]",
                "meta.description: — → \"a probe world\"",
                "next_entity_id: 3 → 9",
                "soundtrack: — → {bpm: 120, path: \"song.ogg\"}",
                "version: 3 → —",
            ]
        );
        assert_eq!(diff.world[4].change(), Change::Removed);
        assert_eq!(diff.world[4].values(), "3 → —");
    }

    #[test]
    fn entities_order_by_numeric_id_then_by_text() {
        let a = world(json!([]));
        let b = world(json!([
            { "id": 10, "name": "ten" },
            { "id": "zed", "name": "named" },
            { "id": 2, "name": "two" },
            { "id": 1, "name": "one" },
            { "name": "anonymous" }
        ]));
        let diff = diff_worlds(&a, &b);
        let ids: Vec<&str> = diff.entities.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "2", "10", "\"zed\"", "#4"]);
        assert!(diff.entities.iter().all(|e| e.change == Change::Added));
    }

    #[test]
    fn anything_but_an_object_is_an_empty_world() {
        assert!(diff_worlds(&json!(null), &json!([1, 2])).is_empty());
        let diff = diff_worlds(
            &json!({}),
            &json!({ "entities": [{ "id": 1, "name": "x" }] }),
        );
        assert_eq!(diff.entities.len(), 1);
        assert_eq!(diff.entities[0].change, Change::Added);
        let diff = diff_worlds(
            &json!({ "entities": "not a list" }),
            &json!({ "entities": [] }),
        );
        assert!(diff.is_empty());
    }

    #[test]
    fn renders_short() {
        // The float that came through f32 prints as the f32 wrote it.
        let widened = Value::from(f64::from(2.4f32));
        assert_eq!(render(&widened), "2.4");
        assert_eq!(render(&json!(2.4)), "2.4");
        assert_eq!(render(&json!(1.0)), "1");
        assert_eq!(render(&json!(-7)), "-7");
        assert_eq!(render(&json!(12000.0)), "12000");
        assert_eq!(render(&json!("maya")), "\"maya\"");
        assert_eq!(render(&json!(true)), "true");
        assert_eq!(render(&json!(null)), "null");
        assert_eq!(
            render(&json!({ "a": [1, { "b": null }] })),
            "{a: [1, {b: null}]}"
        );

        let long: Vec<u64> = (0..1000).collect();
        let text = render(&json!(long));
        assert!(text.ends_with('…'), "{text}");
        assert_eq!(text.chars().count(), MAX_RENDER + 1);
        let wide = render(&json!("é".repeat(200)));
        assert_eq!(wide.chars().count(), MAX_RENDER + 1);
    }
}
