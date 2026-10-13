//! Defer the game's hidden developer log layout, retaining its raw history,
//! error array, trace calls, and visible logging. Both UI reveal paths and the
//! public TextField getter synchronize history before it is displayed/read.
//! This is a game-specific patch, not a change to Flash TextField semantics.

use super::monster_birth::{encode, qualified, read_ops, refers_to};
use super::multiname_local_name;
use swf::avm2::types::{
    AbcFile, Index, MethodBody, MethodFlags, Multiname, Namespace, Op, TraitKind,
};

type Ops = Vec<(usize, usize, Op)>;

fn branch(op: &mut Op) -> Option<&mut i32> {
    match op {
        Op::Jump { offset }
        | Op::IfTrue { offset }
        | Op::IfFalse { offset }
        | Op::IfEq { offset }
        | Op::IfNe { offset }
        | Op::IfGe { offset }
        | Op::IfGt { offset }
        | Op::IfLe { offset }
        | Op::IfLt { offset }
        | Op::IfNge { offset }
        | Op::IfNgt { offset }
        | Op::IfNle { offset }
        | Op::IfNlt { offset }
        | Op::IfStrictEq { offset }
        | Op::IfStrictNe { offset } => Some(offset),
        _ => None,
    }
}

// Branches to the insertion boundary must execute the inserted instructions.
// Branch widths are fixed (s24). Unknown switch tables are rejected unchanged.
fn insert(body: &MethodBody, at: usize, ops: &[Op]) -> Option<MethodBody> {
    let parsed = read_ops(&body.code)?;
    let boundary = |n: usize| n == body.code.len() || parsed.iter().any(|p| p.0 == n);
    if !boundary(at) {
        return None;
    }
    let extra = encode(ops).0;
    let map = |n: usize| n + if n > at { extra.len() } else { 0 };
    let mut result = body.clone();
    result.code.clear();
    for &(start, end, ref original) in &parsed {
        let mut op = original.clone();
        // Offsets below refer to the original bytes; do not silently canonicalize
        // a differently encoded opcode and invalidate the relocation map.
        if encode(&[op.clone()]).0.len() != end - start {
            return None;
        }
        if start == at {
            result.code.extend_from_slice(&extra);
        }
        if matches!(op, Op::LookupSwitch(_)) {
            return None;
        }
        if let Some(offset) = branch(&mut op) {
            let target = usize::try_from(end as i64 + i64::from(*offset)).ok()?;
            if !boundary(target) {
                return None;
            }
            let new_end = end + if start >= at { extra.len() } else { 0 };
            let distance = map(target) as i64 - new_end as i64;
            if !(-8_388_608..=8_388_607).contains(&distance) {
                return None;
            }
            *offset = distance as i32;
        }
        result.code.extend(encode(&[op]).0);
    }
    if at == body.code.len() {
        result.code.extend_from_slice(&extra);
    }
    for e in &mut result.exceptions {
        for offset in [&mut e.from_offset, &mut e.to_offset, &mut e.target_offset] {
            if !boundary(*offset as usize) {
                return None;
            }
            *offset = u32::try_from(map(*offset as usize)).ok()?;
        }
    }
    result.max_stack = result.max_stack.max(2);
    Some(result)
}

fn field_end(abc: &AbcFile, ops: &Ops, at: usize, field: Index<Multiname>) -> Option<usize> {
    match ops.get(at)?.2 {
        Op::GetLex { index } if refers_to(abc, index, field) => Some(at + 1),
        Op::GetLocal { index: 0 } => match ops.get(at + 1)?.2 {
            Op::GetProperty { index } if refers_to(abc, index, field) => Some(at + 2),
            _ => None,
        },
        _ => None,
    }
}

fn public_name(abc: &AbcFile, index: Index<Multiname>, name: &[u8]) -> bool {
    if qualified(abc, index, b"", name) {
        return true;
    }
    if multiname_local_name(abc, index) != Some(name) {
        return false;
    }
    let Some(Multiname::Multiname { namespace_set, .. }) = index
        .0
        .checked_sub(1)
        .and_then(|i| abc.constant_pool.multinames.get(i as usize))
    else {
        return false;
    };
    namespace_set.0.checked_sub(1).and_then(|i| abc.constant_pool.namespace_sets.get(i as usize)).is_some_and(|set|
        set.iter().any(|ns| ns.0.checked_sub(1).and_then(|i| abc.constant_pool.namespaces.get(i as usize)).is_some_and(|ns|
            matches!(ns, Namespace::Package(name) if name.0 == 0 || abc.constant_pool.strings.get(name.0 as usize - 1).is_some_and(Vec::is_empty)))))
}

fn property(abc: &AbcFile, name: &[u8]) -> Option<Index<Multiname>> {
    abc.constant_pool
        .multinames
        .iter()
        .enumerate()
        .find_map(|(i, _)| {
            let index = Index::new(i as u32 + 1);
            public_name(abc, index, name).then_some(index)
        })
}

pub(super) fn patch(abc: &mut AbcFile) -> bool {
    let Some(changes) = prepare(abc) else {
        return false;
    };
    for (index, body) in changes {
        abc.method_bodies[index] = body;
    }
    true
}

fn prepare(abc: &AbcFile) -> Option<Vec<(usize, MethodBody)>> {
    let instance = abc
        .instances
        .iter()
        .find(|i| qualified(abc, i.name, b"com.wdcgame.utils", b"SimpleTextLog"))?;
    let slot = |name: &[u8], package: &[u8], ty: &[u8]| {
        instance.traits.iter().find_map(|t| {
        (multiname_local_name(abc, t.name) == Some(name)
            && matches!(t.kind, TraitKind::Slot { type_name, .. } if qualified(abc, type_name, package, ty)))
            .then_some(t.name)
    })
    };
    let txt = slot(b"_txt", b"flash.text", b"TextField")?;
    let history = slot(b"__test", b"", b"String")?;
    let method = |name: &[u8]| {
        instance.traits.iter().find_map(|t| {
            if multiname_local_name(abc, t.name) != Some(name) {
                return None;
            }
            match t.kind {
                TraitKind::Method { method, .. } | TraitKind::Getter { method, .. } => Some(method),
                _ => None,
            }
        })
    };
    let body = |name: &[u8]| {
        let index = abc.methods.get(method(name)?.0 as usize)?.body?.0 as usize;
        let body = abc.method_bodies.get(index)?;
        Some((index, body))
    };
    let log_method = abc.methods.get(method(b"log")?.0 as usize)?;
    if !log_method.params.is_empty()
        || !log_method.flags.contains(MethodFlags::NEED_REST)
        || !qualified(abc, log_method.return_type, b"", b"void")
    {
        return None;
    }
    let (log_index, log) = body(b"log")?;
    let log_ops = read_ops(&log.code)?;
    let visible = property(abc, b"visible")?;
    let text = property(abc, b"text")?;
    let is_property = |op: &Op, name: &[u8]| matches!(op, Op::GetProperty { index } | Op::SetProperty { index } if public_name(abc, *index, name));
    // An existing visibility guard (including our own) is not an original log.
    if log_ops.iter().any(|p| is_property(&p.2, b"visible")) {
        return None;
    }
    // Require the raw-history write and trace; neither is bypassed by the patch.
    if !log_ops.iter().any(|p| matches!(p.2, Op::SetProperty { index } | Op::InitProperty { index } if refers_to(abc, index, history)))
        || !log_ops.iter().any(|p| matches!(p.2, Op::GetLex { index } | Op::FindPropStrict { index } if public_name(abc, index, b"trace"))) {
        return None;
    }
    let appends: Vec<_> = log_ops.iter().enumerate().filter_map(|(i,p)|
        matches!(p.2, Op::CallPropVoid { index, num_args: 1 } | Op::CallProperty { index, num_args: 1 } if public_name(abc, index, b"appendText")).then_some(i)).collect();
    let [append] = appends.as_slice() else {
        return None;
    };
    let argument_start = match log_ops.get(append.checked_sub(1)?)?.2 {
        Op::GetLocal { .. } => append - 1,
        Op::GetSlot { .. }
            if matches!(
                log_ops.get(append.checked_sub(2)?)?.2,
                Op::GetScopeObject { .. }
            ) =>
        {
            append - 2
        }
        _ => return None,
    };
    let start =
        (0..argument_start).find(|&i| field_end(abc, &log_ops, i, txt) == Some(argument_start))?;
    let after_append = if matches!(log_ops[*append].2, Op::CallProperty { .. }) {
        if log_ops.get(append + 1)?.2 != Op::Pop {
            return None;
        }
        append + 2
    } else {
        append + 1
    };
    let next = field_end(abc, &log_ops, after_append, txt)?;
    let next = field_end(abc, &log_ops, next, txt)?;
    if !is_property(&log_ops.get(next)?.2, b"maxScrollV")
        || !matches!(log_ops.get(next + 1)?.2, Op::SetProperty { index } if public_name(abc, index, b"scrollV"))
    {
        return None;
    }
    let begin = log_ops[start].0;
    let end = log_ops[next + 1].1;
    // No old control-flow edge may enter the middle of the UI block.
    for (_, op_end, op) in &log_ops {
        if let Some(offset) = branch(&mut op.clone()) {
            let target = *op_end as i64 + i64::from(*offset);
            if target > begin as i64 && target < end as i64 {
                return None;
            }
        }
    }
    if log
        .exceptions
        .iter()
        .any(|e| (e.from_offset as usize) < end && (e.to_offset as usize) > begin)
    {
        return None;
    }
    let guard = [
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: txt },
        Op::GetProperty { index: visible },
        Op::IfFalse {
            offset: (end - begin) as i32,
        },
    ];
    let log = insert(log, begin, &guard)?;
    let sync = [
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: txt },
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: history },
        Op::SetProperty { index: text },
    ];

    let mut sync_hidden = vec![
        Op::GetLocal { index: 0 },
        Op::GetProperty { index: txt },
        Op::GetProperty { index: visible },
        Op::IfTrue {
            offset: encode(&sync).0.len() as i32,
        },
    ];
    sync_hidden.extend(sync);

    // Keyboard reveal already reconstructs the complete raw history.
    let (_, key) = body(b"__keyup")?;
    let key_ops = read_ops(&key.code)?;
    if !(0..key_ops.len()).any(|i| field_end(abc, &key_ops, i, history).is_some_and(|end|
        matches!(key_ops.get(end).map(|p| &p.2), Some(Op::SetProperty { index }) if public_name(abc, *index, b"text")))) {
        return None;
    }
    // The mouse reveal previously just toggled visibility: synchronize there too.
    let (mouse_index, mouse) = body(b"onMouse")?;
    let mouse_ops = read_ops(&mouse.code)?;
    let reveals: Vec<_> = (0..mouse_ops.len()).filter(|&i| field_end(abc, &mouse_ops, i, txt).is_some_and(|end|
        matches!(mouse_ops.get(end).map(|p| &p.2), Some(Op::PushTrue))
        && matches!(mouse_ops.get(end + 1).map(|p| &p.2), Some(Op::SetProperty { index }) if public_name(abc, *index, b"visible")))).collect();
    let [reveal] = reveals.as_slice() else {
        return None;
    };
    let mouse = insert(mouse, mouse_ops[*reveal].0, &sync_hidden)?;

    // Also cover code which retrieves the hidden TextField directly.
    let (getter_index, getter) = body(b"txt")?;
    let getter_ops = read_ops(&getter.code)?;
    if getter_ops.len() < 4
        || getter_ops[0].2 != (Op::GetLocal { index: 0 })
        || getter_ops[1].2 != Op::PushScope
    {
        return None;
    }
    let return_at = field_end(abc, &getter_ops, 2, txt)?;
    if return_at + 1 != getter_ops.len() || getter_ops[return_at].2 != Op::ReturnValue {
        return None;
    }
    let getter = insert(getter, getter_ops[2].0, &sync_hidden)?;
    Some(vec![
        (log_index, log),
        (mouse_index, mouse),
        (getter_index, getter),
    ])
}
