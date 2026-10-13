//! A guarded fast path for ZM5's existing id-indexed spawn dictionary.
//!
//! The reviewed manager inserts births under `birth.id` and removes them by id.
//! Its lookup nevertheless enumerates all values inside nested death checks.
//! Check the exact lookup/control flow before prepending a key lookup; retain
//! the original bytecode for misses or a candidate whose id no longer matches.
//! No new cache, death result, spawn trigger or logging rule is introduced.

use super::{AbcReader, AbcWriter, multiname_local_name};
use swf::{
    avm2::types::{AbcFile, Index, MethodFlags, Multiname, Namespace, Op, TraitKind},
    extensions::ReadSwfExt,
};

fn string(abc: &AbcFile, index: Index<String>) -> Option<&[u8]> {
    if index.0 == 0 {
        Some(b"")
    } else {
        abc.constant_pool
            .strings
            .get(index.0 as usize - 1)
            .map(Vec::as_slice)
    }
}

pub(super) fn qualified(
    abc: &AbcFile,
    index: Index<Multiname>,
    package: &[u8],
    name: &[u8],
) -> bool {
    let Some(Multiname::QName {
        namespace,
        name: local,
    }) = index
        .0
        .checked_sub(1)
        .and_then(|i| abc.constant_pool.multinames.get(i as usize))
    else {
        return false;
    };
    let Some(Namespace::Package(ns)) = namespace
        .0
        .checked_sub(1)
        .and_then(|i| abc.constant_pool.namespaces.get(i as usize))
    else {
        return false;
    };
    string(abc, *ns) == Some(package) && string(abc, *local) == Some(name)
}

pub(super) fn refers_to(
    abc: &AbcFile,
    lookup: Index<Multiname>,
    declared: Index<Multiname>,
) -> bool {
    if lookup == declared {
        return true;
    }
    let pool = &abc.constant_pool;
    let Some(Multiname::QName { namespace, name }) = declared
        .0
        .checked_sub(1)
        .and_then(|i| pool.multinames.get(i as usize))
    else {
        return false;
    };
    let Some(Multiname::Multiname {
        namespace_set,
        name: lookup_name,
    }) = lookup
        .0
        .checked_sub(1)
        .and_then(|i| pool.multinames.get(i as usize))
    else {
        return false;
    };
    string(abc, *name) == string(abc, *lookup_name)
        && namespace_set
            .0
            .checked_sub(1)
            .and_then(|i| pool.namespace_sets.get(i as usize))
            .is_some_and(|set| set.contains(namespace))
}

pub(super) fn read_ops(code: &[u8]) -> Option<Vec<(usize, usize, Op)>> {
    let mut reader = AbcReader::new(code);
    let mut ops = Vec::new();
    while !reader.as_slice().is_empty() {
        let start = code.len() - reader.as_slice().len();
        let op = reader.read_op().ok()?;
        ops.push((start, code.len() - reader.as_slice().len(), op));
    }
    Some(ops)
}

pub(super) fn encode(ops: &[Op]) -> (Vec<u8>, Vec<usize>) {
    let mut code = Vec::new();
    let mut ends = Vec::new();
    for op in ops {
        AbcWriter::new(&mut code)
            .write_op(op)
            .expect("write opcode to Vec");
        ends.push(code.len());
    }
    (code, ends)
}

pub(super) fn patch(abc: &mut AbcFile) -> bool {
    patch_checked(abc).is_some()
}

fn patch_checked(abc: &mut AbcFile) -> Option<()> {
    let instance = abc.instances.iter().find(|instance| {
        qualified(
            abc,
            instance.name,
            b"com.wdcgame.engine.managers",
            b"ManCreateManager",
        )
    })?;
    let field = instance.traits.iter().find(|t| {
        multiname_local_name(abc, t.name) == Some(b"_births")
            && matches!(t.kind, TraitKind::Slot { type_name, .. }
                if qualified(abc, type_name, b"flash.utils", b"Dictionary"))
    })?;
    let method_index = instance.traits.iter().find_map(|t| {
        if !qualified(abc, t.name, b"", b"getMonsterBirthVo") {
            return None;
        }
        match t.kind {
            TraitKind::Method { method, .. } => Some(method),
            _ => None,
        }
    })?;
    let method = abc.methods.get(method_index.0 as usize)?;
    if method.params.len() != 1
        || !qualified(abc, method.params[0].kind, b"", b"int")
        || method.params[0].default_value.is_some()
        || !(method.flags - MethodFlags::HAS_PARAM_NAMES).is_empty()
        || !qualified(
            abc,
            method.return_type,
            b"com.wdcgame.engine.model",
            b"MonsterBirthVO",
        )
    {
        return None;
    }
    let birth_type = method.return_type;
    let body_index = method.body?.0 as usize;
    let body = abc.method_bodies.get(body_index)?;
    if body.max_stack != 2
        || body.method != method_index
        || body.num_locals != 5
        || !body.exceptions.is_empty()
        || !body.traits.is_empty()
        || body.max_scope_depth != body.init_scope_depth + 1
    {
        return None;
    }
    let parsed = read_ops(&body.code)?;
    let properties: Vec<_> = parsed
        .iter()
        .filter_map(|(_, _, op)| match op {
            Op::GetProperty { index } => Some(*index),
            _ => None,
        })
        .collect();
    let [births, id] = properties.as_slice() else {
        return None;
    };
    // The production compiler emits the slot's exact QName; ASC's fixture emits
    // a namespace-set lookup. The fallback keeps that original lookup intact.
    let declared_id = abc.instances.iter().find(|i| qualified(abc, i.name, b"com.wdcgame.engine.model", b"MonsterBirthVO"))?
        .traits.iter().find(|t| qualified(abc, t.name, b"", b"id")
            && matches!(t.kind, TraitKind::Slot { type_name, .. } if qualified(abc, type_name, b"", b"int")))?.name;
    if !refers_to(abc, *births, field.name) || !refers_to(abc, *id, declared_id) {
        return None;
    }
    // Verify the reviewed builder still stores values under their own id.
    let builder = instance.traits.iter().find_map(|t| {
        if !qualified(abc, t.name, b"", b"buildBirthFromObjectVo") {
            return None;
        }
        match t.kind {
            TraitKind::Method { method, .. } => Some(method),
            _ => None,
        }
    })?;
    let builder_body = abc.methods.get(builder.0 as usize)?.body?;
    let builder_ops = read_ops(&abc.method_bodies.get(builder_body.0 as usize)?.code)?;
    let indexed_insert = builder_ops.windows(5).any(|ops| {
        match (&ops[0].2, &ops[1].2, &ops[2].2, &ops[3].2, &ops[4].2) {
            (
                Op::GetLex { index: dict } | Op::GetProperty { index: dict },
                Op::GetLocal { index: a },
                Op::GetProperty { index: key },
                Op::GetLocal { index: b },
                Op::SetProperty { index: property },
            ) => {
                refers_to(abc, *dict, field.name)
                    && refers_to(abc, *key, declared_id)
                    && a == b
                    && property
                        .0
                        .checked_sub(1)
                        .and_then(|i| abc.constant_pool.multinames.get(i as usize))
                        .is_some_and(|n| matches!(n, Multiname::MultinameL { .. }))
            }
            _ => false,
        }
    });
    if !indexed_insert {
        return None;
    }
    let has_next: Vec<_> = parsed
        .iter()
        .enumerate()
        .filter_map(|(i, (_, _, op))| match op {
            Op::HasNext2 {
                object_register,
                index_register,
            } => Some((i, *object_register, *index_register)),
            _ => None,
        })
        .collect();
    let [(test, object, index)] = has_next.as_slice() else {
        return None;
    };
    let asc = (*object, *index) == (4, 3);
    if !asc && (*object, *index) != (3, 4) {
        return None;
    }
    let mut expected = vec![Op::GetLocal { index: 0 }, Op::PushScope];
    if asc {
        expected.extend([
            Op::PushNull,
            Op::Coerce { index: birth_type },
            Op::SetLocal { index: 2 },
        ]);
    }
    expected.extend([
        Op::PushByte { value: 0 },
        Op::SetLocal { index: *index },
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: *births },
    ]);
    if asc {
        expected.push(Op::CoerceA);
    }
    expected.extend([Op::SetLocal { index: *object }, Op::Jump { offset: 0 }]);
    let loop_start = expected.len();
    expected.extend([
        Op::Label,
        Op::GetLocal { index: *object },
        Op::GetLocal { index: *index },
        Op::NextValue,
        Op::Coerce { index: birth_type },
        Op::SetLocal { index: 2 },
        Op::GetLocal { index: 2 },
        Op::GetProperty { index: *id },
        Op::GetLocal { index: 1 },
        Op::IfNe { offset: 0 },
        Op::GetLocal { index: 2 },
        Op::ReturnValue,
        Op::HasNext2 {
            object_register: *object,
            index_register: *index,
        },
        Op::IfTrue { offset: 0 },
    ]);
    if asc {
        expected.extend([Op::Kill { index: *object }, Op::Kill { index: *index }]);
    }
    expected.extend([Op::PushNull, Op::ReturnValue]);
    if expected.len() != parsed.len() {
        return None;
    }
    for ((_, end, actual), expected) in parsed.iter().zip(&expected) {
        let target = match (actual, expected) {
            (Op::Jump { offset }, Op::Jump { .. }) | (Op::IfNe { offset }, Op::IfNe { .. }) => {
                Some((*offset, *test))
            }
            (Op::IfTrue { offset }, Op::IfTrue { .. }) => Some((*offset, loop_start)),
            _ if actual == expected => None,
            _ => return None,
        };
        if let Some((offset, op_index)) = target
            && (*end as i64 + i64::from(offset)) != parsed[op_index].0 as i64
        {
            return None;
        }
    }
    // Use the already-validated private slot QName, rather than a broad namespace
    // set, and add a public late-name property for the dictionary's integer key.
    let births = field.name;
    let id = *id;
    let public_ns = Index::new(abc.constant_pool.namespaces.len() as u32 + 1);
    let ns_set = Index::new(abc.constant_pool.namespace_sets.len() as u32 + 1);
    let lookup = Index::new(abc.constant_pool.multinames.len() as u32 + 1);
    let mut prefix = vec![
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: births },
        Op::GetLocal { index: 1 },
        Op::GetProperty { index: lookup },
        Op::Coerce { index: birth_type },
        Op::SetLocal { index: 2 },
        Op::GetLocal { index: 2 },
        Op::IfFalse { offset: 0 },
        Op::GetLocal { index: 2 },
        Op::GetProperty { index: id },
        Op::GetLocal { index: 1 },
        Op::IfNe { offset: 0 },
        Op::GetLocal { index: 2 },
        Op::ReturnValue,
    ];
    let (bytes, ends) = encode(&prefix);
    prefix[7] = Op::IfFalse {
        offset: (bytes.len() - ends[7]) as i32,
    };
    prefix[11] = Op::IfNe {
        offset: (bytes.len() - ends[11]) as i32,
    };
    let (mut code, _) = encode(&prefix);
    code.extend_from_slice(&body.code); // all old relative jumps remain unchanged
    abc.constant_pool
        .namespaces
        .push(Namespace::Package(Index::new(0)));
    abc.constant_pool.namespace_sets.push(vec![public_ns]);
    abc.constant_pool.multinames.push(Multiname::MultinameL {
        namespace_set: ns_set,
    });
    abc.method_bodies[body_index].code = code;
    Some(())
}
