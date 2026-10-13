use std::io::{Read, Write};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use swf::avm2::{
    read::Reader as AbcReader,
    types::{AbcFile, DefaultValue, Index, Multiname, Op, TraitKind},
    write::Writer as AbcWriter,
};
use zm_core::{GameKind, Result, ZmError};

mod monster_birth;
mod text_log;

const SYMBOL_CLASS: u16 = 76;
const SHOW_FRAME: u16 = 1;
const DO_ABC: u16 = 82;

pub fn inject_bridge(source: &[u8], abc: &[u8], game: GameKind) -> Result<Vec<u8>> {
    if source.len() < 12 {
        return Err(ZmError::Asset("SWF文件过短".into()));
    }
    let version = source[3];
    let mut body = match &source[..3] {
        b"FWS" => source[8..].to_vec(),
        b"CWS" => {
            let mut decoded = Vec::new();
            ZlibDecoder::new(&source[8..])
                .read_to_end(&mut decoded)
                .map_err(|e| ZmError::Asset(format!("解压SWF失败：{e}")))?;
            decoded
        }
        b"ZWS" => decode_zws(source)?,
        _ => return Err(ZmError::Asset("无效SWF签名".into())),
    };

    if game == GameKind::Zm5 {
        // Passing an explicit resource path skips ZM5's official bootstrap branch that selects
        // release mode, leaving the client in local-development mode with its GM panel enabled.
        patch_zm5_runtime(&mut body)?;
    }

    let class_name = game.profile().bridge_class;
    let tags_start = frame_header_len(&body)?;
    let mut cursor = tags_start;
    let mut replacement = None;
    while cursor + 2 <= body.len() {
        let record = u16::from_le_bytes([body[cursor], body[cursor + 1]]);
        let code = record >> 6;
        let short_len = (record & 0x3f) as usize;
        let (header_len, body_len) = if short_len == 0x3f {
            if cursor + 6 > body.len() {
                return Err(ZmError::Asset("SWF长标签头损坏".into()));
            }
            (
                6,
                u32::from_le_bytes([
                    body[cursor + 2],
                    body[cursor + 3],
                    body[cursor + 4],
                    body[cursor + 5],
                ]) as usize,
            )
        } else {
            (2, short_len)
        };
        let start = cursor + header_len;
        let end = start
            .checked_add(body_len)
            .ok_or_else(|| ZmError::Asset("SWF标签长度溢出".into()))?;
        if end > body.len() {
            return Err(ZmError::Asset("SWF标签越界".into()));
        }
        if code == SYMBOL_CLASS {
            replacement = Some(rewrite_symbol_class(&body[start..end], class_name)?);
            let replacement = replacement.as_ref().unwrap();
            let mut tag = Vec::with_capacity(replacement.len() + 6);
            if replacement.len() < 0x3f {
                tag.extend_from_slice(
                    &((SYMBOL_CLASS << 6) | replacement.len() as u16).to_le_bytes(),
                );
            } else {
                tag.extend_from_slice(&((SYMBOL_CLASS << 6) | 0x3f).to_le_bytes());
                tag.extend_from_slice(&(replacement.len() as u32).to_le_bytes());
            }
            tag.extend_from_slice(replacement);
            body.splice(cursor..end, tag);
            break;
        }
        cursor = end;
        if code == 0 {
            break;
        }
    }
    if replacement.is_none() {
        return Err(ZmError::Asset("SWF中没有SymbolClass标签".into()));
    }

    let tags_start = frame_header_len(&body)?;
    let insertion = first_tag_offset(&body, tags_start, SHOW_FRAME)?
        .ok_or_else(|| ZmError::Asset("SWF中没有首帧标签".into()))?;
    body.splice(insertion..insertion, do_abc_tag(abc));

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&body)
        .map_err(|e| ZmError::Asset(format!("压缩SWF失败：{e}")))?;
    let compressed = encoder
        .finish()
        .map_err(|e| ZmError::Asset(format!("压缩SWF失败：{e}")))?;
    let mut output = Vec::with_capacity(compressed.len() + 8);
    output.extend_from_slice(b"CWS");
    output.push(version);
    output.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
    output.extend_from_slice(&compressed);
    Ok(output)
}

fn patch_zm5_runtime(body: &mut Vec<u8>) -> Result<()> {
    let mut cursor = frame_header_len(body)?;
    while cursor + 2 <= body.len() {
        let (code, header_len, payload_len) = tag_header(body, cursor)?;
        let start = cursor + header_len;
        let end = start
            .checked_add(payload_len)
            .ok_or_else(|| ZmError::Asset("SWF标签长度溢出".into()))?;
        if end > body.len() {
            return Err(ZmError::Asset("SWF标签越界".into()));
        }

        if code == DO_ABC {
            let payload = &body[start..end];
            let main_program = payload
                .windows(b"CreateGmCmd".len())
                .any(|p| p == b"CreateGmCmd");
            let birth_manager = payload
                .windows(b"ManCreateManager".len())
                .any(|p| p == b"ManCreateManager");
            let text_logger = payload
                .windows(b"SimpleTextLog".len())
                .any(|p| p == b"SimpleTextLog");
            if main_program || birth_manager || text_logger {
                let abc_offset = do_abc_data_offset(payload)?;
                let mut abc = AbcReader::new(&payload[abc_offset..])
                    .read()
                    .map_err(|error| ZmError::Asset(format!("解析造梦西游5主程序失败：{error}")))?;
                if main_program {
                    let outcome = patch_zm5_abc(&mut abc)?;
                    if !outcome.release_mode || !outcome.gm_command {
                        return Err(ZmError::Asset(
                            "造梦西游5主程序结构已变化，无法安全关闭开发工具".into(),
                        ));
                    }
                }
                let birth_lookup = monster_birth::patch(&mut abc);
                if birth_manager {
                    tracing::info!(target: "zm_perf", applied = birth_lookup, "ZM5 guarded birth lookup fast path");
                }
                let hidden_log = text_log::patch(&mut abc);
                if text_logger {
                    tracing::info!(target: "zm_perf", applied = hidden_log, "ZM5 deferred hidden text log");
                }
                if !main_program && !birth_lookup && !hidden_log {
                    cursor = end;
                    continue;
                }

                let mut encoded_abc = Vec::with_capacity(payload.len() - abc_offset);
                AbcWriter::new(&mut encoded_abc)
                    .write(abc)
                    .map_err(|error| ZmError::Asset(format!("重写造梦西游5主程序失败：{error}")))?;
                let mut replacement = Vec::with_capacity(abc_offset + encoded_abc.len());
                replacement.extend_from_slice(&payload[..abc_offset]);
                replacement.extend_from_slice(&encoded_abc);
                let tag = encode_tag(DO_ABC, &replacement);
                let next = cursor + tag.len();
                body.splice(cursor..end, tag);
                cursor = next;
                continue;
            }
        }

        cursor = end;
        if code == 0 {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Zm5PatchOutcome {
    release_mode: bool,
    gm_command: bool,
}

fn patch_zm5_abc(abc: &mut AbcFile) -> Result<Zm5PatchOutcome> {
    let release_value = abc
        .constant_pool
        .ints
        .iter()
        .position(|value| *value == 4)
        .map(|index| Index::new(index as u32 + 1))
        .ok_or_else(|| ZmError::Asset("造梦西游5主程序缺少正式版运行标记".into()))?;
    let mut outcome = Zm5PatchOutcome::default();
    let mut gm_method = None;

    for (index, instance) in abc.instances.iter().enumerate() {
        let class_name = multiname_local_name(abc, instance.name);
        if class_name == Some(b"GameData") {
            if let Some(class) = abc.classes.get_mut(index) {
                for class_trait in &mut class.traits {
                    if multiname_local_name_from_pool(
                        &abc.constant_pool.multinames,
                        &abc.constant_pool.strings,
                        class_trait.name,
                    ) == Some(b"_gameVersion")
                        && let TraitKind::Slot { value, .. } = &mut class_trait.kind
                    {
                        *value = Some(DefaultValue::Int(release_value));
                        outcome.release_mode = true;
                    }
                }
            }
        } else if class_name == Some(b"CreateGmCmd") {
            for instance_trait in &instance.traits {
                if multiname_local_name(abc, instance_trait.name) == Some(b"execute")
                    && let TraitKind::Method { method, .. } = instance_trait.kind
                {
                    gm_method = Some(method);
                }
            }
        }
    }

    if let Some(method) = gm_method {
        let body_index = abc
            .methods
            .get(method.0 as usize)
            .and_then(|method| method.body)
            .ok_or_else(|| ZmError::Asset("造梦西游5 GM命令缺少方法体".into()))?;
        let body = abc
            .method_bodies
            .get_mut(body_index.0 as usize)
            .ok_or_else(|| ZmError::Asset("造梦西游5 GM命令方法体越界".into()))?;
        let mut code = Vec::with_capacity(1);
        AbcWriter::new(&mut code)
            .write_op(&Op::ReturnVoid)
            .map_err(|error| ZmError::Asset(format!("生成造梦西游5补丁失败：{error}")))?;
        body.code = code;
        body.max_stack = 0;
        body.max_scope_depth = body.init_scope_depth;
        body.exceptions.clear();
        body.traits.clear();
        outcome.gm_command = true;
    }

    Ok(outcome)
}

fn multiname_local_name(abc: &AbcFile, index: Index<Multiname>) -> Option<&[u8]> {
    multiname_local_name_from_pool(
        &abc.constant_pool.multinames,
        &abc.constant_pool.strings,
        index,
    )
}

fn multiname_local_name_from_pool<'a>(
    multinames: &'a [Multiname],
    strings: &'a [Vec<u8>],
    index: Index<Multiname>,
) -> Option<&'a [u8]> {
    let multiname = index
        .0
        .checked_sub(1)
        .and_then(|index| multinames.get(index as usize))?;
    let name = match multiname {
        Multiname::QName { name, .. }
        | Multiname::QNameA { name, .. }
        | Multiname::RTQName { name }
        | Multiname::RTQNameA { name }
        | Multiname::Multiname { name, .. }
        | Multiname::MultinameA { name, .. } => *name,
        _ => return None,
    };
    name.0
        .checked_sub(1)
        .and_then(|index| strings.get(index as usize))
        .map(Vec::as_slice)
}

fn do_abc_data_offset(payload: &[u8]) -> Result<usize> {
    if payload.len() < 5 {
        return Err(ZmError::Asset("DoABC标签过短".into()));
    }
    payload[4..]
        .iter()
        .position(|byte| *byte == 0)
        .map(|name_len| 5 + name_len)
        .ok_or_else(|| ZmError::Asset("DoABC名称未终止".into()))
}

fn tag_header(body: &[u8], cursor: usize) -> Result<(u16, usize, usize)> {
    let record = u16::from_le_bytes([body[cursor], body[cursor + 1]]);
    let code = record >> 6;
    let short_len = (record & 0x3f) as usize;
    if short_len != 0x3f {
        return Ok((code, 2, short_len));
    }
    if cursor + 6 > body.len() {
        return Err(ZmError::Asset("SWF长标签头损坏".into()));
    }
    Ok((
        code,
        6,
        u32::from_le_bytes([
            body[cursor + 2],
            body[cursor + 3],
            body[cursor + 4],
            body[cursor + 5],
        ]) as usize,
    ))
}

fn encode_tag(code: u16, payload: &[u8]) -> Vec<u8> {
    let mut tag = Vec::with_capacity(payload.len() + 6);
    if payload.len() < 0x3f {
        tag.extend_from_slice(&((code << 6) | payload.len() as u16).to_le_bytes());
    } else {
        tag.extend_from_slice(&((code << 6) | 0x3f).to_le_bytes());
        tag.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    tag.extend_from_slice(payload);
    tag
}

fn first_tag_offset(body: &[u8], mut cursor: usize, wanted: u16) -> Result<Option<usize>> {
    while cursor + 2 <= body.len() {
        let record = u16::from_le_bytes([body[cursor], body[cursor + 1]]);
        let code = record >> 6;
        if code == wanted {
            return Ok(Some(cursor));
        }
        let short_len = (record & 0x3f) as usize;
        let (header_len, body_len) = if short_len == 0x3f {
            if cursor + 6 > body.len() {
                return Err(ZmError::Asset("SWF长标签头损坏".into()));
            }
            (
                6,
                u32::from_le_bytes([
                    body[cursor + 2],
                    body[cursor + 3],
                    body[cursor + 4],
                    body[cursor + 5],
                ]) as usize,
            )
        } else {
            (2, short_len)
        };
        cursor = cursor
            .checked_add(header_len + body_len)
            .ok_or_else(|| ZmError::Asset("SWF标签长度溢出".into()))?;
        if cursor > body.len() {
            return Err(ZmError::Asset("SWF标签越界".into()));
        }
        if code == 0 {
            break;
        }
    }
    Ok(None)
}

fn do_abc_tag(abc: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(abc.len() + 14);
    payload.extend_from_slice(&0_u32.to_le_bytes());
    payload.extend_from_slice(b"ZM-LINUX session bridge\0");
    payload.extend_from_slice(abc);
    let mut tag = Vec::with_capacity(payload.len() + 6);
    tag.extend_from_slice(&((DO_ABC << 6) | 0x3f).to_le_bytes());
    tag.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    tag.extend_from_slice(&payload);
    tag
}

fn decode_zws(source: &[u8]) -> Result<Vec<u8>> {
    if source.len() < 17 {
        return Err(ZmError::Asset("LZMA SWF头损坏".into()));
    }
    let file_len = u32::from_le_bytes([source[4], source[5], source[6], source[7]]) as u64;
    if file_len < 8 {
        return Err(ZmError::Asset("LZMA SWF长度无效".into()));
    }
    let mut lzma_stream = Vec::with_capacity(source.len() + 8);
    lzma_stream.extend_from_slice(&source[12..17]);
    lzma_stream.extend_from_slice(&(file_len - 8).to_le_bytes());
    lzma_stream.extend_from_slice(&source[17..]);
    let mut decoded = Vec::with_capacity((file_len - 8) as usize);
    lzma_rs::lzma_decompress(&mut lzma_stream.as_slice(), &mut decoded)
        .map_err(|e| ZmError::Asset(format!("解压LZMA SWF失败：{e}")))?;
    Ok(decoded)
}

fn frame_header_len(body: &[u8]) -> Result<usize> {
    let first = *body
        .first()
        .ok_or_else(|| ZmError::Asset("SWF缺少RECT".into()))?;
    let nbits = (first >> 3) as usize;
    let rect_bytes = (5 + nbits * 4).div_ceil(8);
    let length = rect_bytes + 4;
    if length > body.len() {
        return Err(ZmError::Asset("SWF帧头越界".into()));
    }
    Ok(length)
}

fn rewrite_symbol_class(tag: &[u8], class_name: &str) -> Result<Vec<u8>> {
    if tag.len() < 2 {
        return Err(ZmError::Asset("SymbolClass标签过短".into()));
    }
    let count = u16::from_le_bytes([tag[0], tag[1]]);
    let mut input = 2;
    let mut output = Vec::with_capacity(tag.len());
    output.extend_from_slice(&count.to_le_bytes());
    let mut replaced = false;
    for _ in 0..count {
        if input + 2 > tag.len() {
            return Err(ZmError::Asset("SymbolClass条目损坏".into()));
        }
        let id = u16::from_le_bytes([tag[input], tag[input + 1]]);
        input += 2;
        let nul = tag[input..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| ZmError::Asset("SymbolClass名称未终止".into()))?;
        let old_name = &tag[input..input + nul];
        input += nul + 1;
        output.extend_from_slice(&id.to_le_bytes());
        if id == 0 {
            output.extend_from_slice(class_name.as_bytes());
            replaced = true;
        } else {
            output.extend_from_slice(old_name);
        }
        output.push(0);
    }
    if !replaced {
        return Err(ZmError::Asset("SymbolClass中没有根文档类".into()));
    }
    output.extend_from_slice(&tag[input..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swf::avm2::types::{
        Class, ConstantPool, Instance, Method, MethodBody, MethodFlags, Namespace, Trait,
    };

    fn method(body: Option<u32>) -> Method {
        Method {
            name: Index::new(0),
            params: Vec::new(),
            return_type: Index::new(0),
            flags: MethodFlags::empty(),
            body: body.map(Index::new),
        }
    }
    #[test]
    fn rewrites_only_root_entry() {
        let mut tag = vec![2, 0, 0, 0];
        tag.extend_from_slice(b"Main\0");
        tag.extend_from_slice(&7_u16.to_le_bytes());
        tag.extend_from_slice(b"Asset\0");
        let result = rewrite_symbol_class(&tag, "Preload").unwrap();
        assert!(result.windows(8).any(|w| w == b"Preload\0"));
        assert!(result.windows(6).any(|w| w == b"Asset\0"));
        assert!(!result.windows(5).any(|w| w == b"Main\0"));
    }

    #[test]
    fn bridge_abc_tag_preserves_payload() {
        let tag = do_abc_tag(b"ABC bytes");
        assert_eq!(u16::from_le_bytes([tag[0], tag[1]]) >> 6, DO_ABC);
        assert!(tag.windows(9).any(|window| window == b"ABC bytes"));
        assert!(
            tag.windows(24)
                .any(|window| window == b"ZM-LINUX session bridge\0")
        );
    }

    #[test]
    fn patches_document_class_and_inserts_bridge_before_first_frame() {
        let mut body = vec![0x08, 0x00, 0x00, 0x18, 0x01, 0x00];
        let mut symbols = vec![1, 0, 0, 0];
        symbols.extend_from_slice(b"Preload\0");
        body.extend_from_slice(&((SYMBOL_CLASS << 6) | symbols.len() as u16).to_le_bytes());
        body.extend_from_slice(&symbols);
        body.extend_from_slice(&(SHOW_FRAME << 6).to_le_bytes());
        body.extend_from_slice(&0_u16.to_le_bytes());

        let mut source = Vec::new();
        source.extend_from_slice(b"FWS");
        source.push(10);
        source.extend_from_slice(&((body.len() + 8) as u32).to_le_bytes());
        source.extend_from_slice(&body);

        let output = inject_bridge(&source, b"bridge-abc", GameKind::Zm4).unwrap();
        let mut decoded = Vec::new();
        ZlibDecoder::new(&output[8..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert!(
            decoded
                .windows(b"ZmLinuxZm4Bridge".len())
                .any(|window| window == b"ZmLinuxZm4Bridge")
        );
        let abc_at = decoded
            .windows(10)
            .position(|window| window == b"bridge-abc")
            .unwrap();
        let show_frame_at =
            first_tag_offset(&decoded, frame_header_len(&decoded).unwrap(), SHOW_FRAME)
                .unwrap()
                .unwrap();
        assert!(abc_at < show_frame_at);
    }

    #[test]
    fn zm5_patch_selects_release_mode_and_disables_gm_command() {
        let qname = |name| Multiname::QName {
            namespace: Index::new(1),
            name: Index::new(name),
        };
        let mut abc = AbcFile {
            major_version: 46,
            minor_version: 16,
            constant_pool: ConstantPool {
                ints: vec![2, 4],
                uints: Vec::new(),
                doubles: Vec::new(),
                strings: vec![
                    b"GameData".to_vec(),
                    b"_gameVersion".to_vec(),
                    b"CreateGmCmd".to_vec(),
                    b"execute".to_vec(),
                ],
                namespaces: vec![Namespace::Package(Index::new(0))],
                namespace_sets: Vec::new(),
                multinames: vec![qname(1), qname(2), qname(3), qname(4)],
            },
            methods: vec![method(None), method(None), method(Some(0))],
            metadata: Vec::new(),
            instances: vec![
                Instance {
                    name: Index::new(1),
                    super_name: Index::new(0),
                    is_sealed: true,
                    is_final: false,
                    is_interface: false,
                    protected_namespace: None,
                    interfaces: Vec::new(),
                    init_method: Index::new(0),
                    traits: Vec::new(),
                },
                Instance {
                    name: Index::new(3),
                    super_name: Index::new(0),
                    is_sealed: true,
                    is_final: false,
                    is_interface: false,
                    protected_namespace: None,
                    interfaces: Vec::new(),
                    init_method: Index::new(1),
                    traits: vec![Trait {
                        name: Index::new(4),
                        kind: TraitKind::Method {
                            disp_id: 0,
                            method: Index::new(2),
                        },
                        metadata: Vec::new(),
                        is_final: false,
                        is_override: false,
                    }],
                },
            ],
            classes: vec![
                Class {
                    init_method: Index::new(0),
                    traits: vec![Trait {
                        name: Index::new(2),
                        kind: TraitKind::Slot {
                            slot_id: 1,
                            type_name: Index::new(0),
                            value: Some(DefaultValue::Int(Index::new(1))),
                        },
                        metadata: Vec::new(),
                        is_final: false,
                        is_override: false,
                    }],
                },
                Class {
                    init_method: Index::new(1),
                    traits: Vec::new(),
                },
            ],
            scripts: Vec::new(),
            method_bodies: vec![MethodBody {
                method: Index::new(2),
                max_stack: 2,
                num_locals: 2,
                init_scope_depth: 1,
                max_scope_depth: 3,
                code: vec![0x24, 0x01, 0x47],
                exceptions: Vec::new(),
                traits: Vec::new(),
            }],
        };

        let outcome = patch_zm5_abc(&mut abc).unwrap();

        assert!(outcome.release_mode);
        assert!(outcome.gm_command);
        let TraitKind::Slot { value, .. } = &abc.classes[0].traits[0].kind else {
            panic!("expected game version slot")
        };
        assert_eq!(*value, Some(DefaultValue::Int(Index::new(2))));
        assert_eq!(abc.method_bodies[0].code, vec![0x47]);
        assert_eq!(abc.method_bodies[0].max_stack, 0);
        assert_eq!(abc.method_bodies[0].max_scope_depth, 1);
    }
}

#[cfg(test)]
mod birth_lookup_tests {
    use super::*;
    use ruffle_core::{LoadBehavior, PlayerBuilder, backend::log::LogBackend, tag_utils::SwfMovie};
    use std::sync::{Arc, Mutex};

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/MonsterBirthLookup.swf");

    pub(super) fn decode(source: &[u8]) -> Vec<u8> {
        match &source[..3] {
            b"FWS" => source[8..].to_vec(),
            b"CWS" => {
                let mut body = Vec::new();
                ZlibDecoder::new(&source[8..])
                    .read_to_end(&mut body)
                    .unwrap();
                body
            }
            _ => panic!("unexpected fixture compression"),
        }
    }

    fn abc_with_lookup(source: &[u8]) -> AbcFile {
        abc_with_class(source, b"ManCreateManager")
    }

    pub(super) fn abc_with_class(source: &[u8], class: &[u8]) -> AbcFile {
        let body = decode(source);
        let mut cursor = frame_header_len(&body).unwrap();
        while cursor + 2 <= body.len() {
            let (kind, header, length) = tag_header(&body, cursor).unwrap();
            let payload = &body[cursor + header..cursor + header + length];
            if kind == DO_ABC && payload.windows(class.len()).any(|p| p == class) {
                return AbcReader::new(&payload[do_abc_data_offset(payload).unwrap()..])
                    .read()
                    .unwrap();
            }
            cursor += header + length;
        }
        panic!("fixture manager missing");
    }

    pub(super) fn patch_swf(source: &[u8]) -> Vec<u8> {
        let mut body = decode(source);
        patch_zm5_runtime(&mut body).unwrap();
        let mut result = b"FWS".to_vec();
        result.push(source[3]);
        result.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        result.extend(body);
        result
    }

    struct Log(Arc<Mutex<Vec<String>>>);
    impl LogBackend for Log {
        fn avm_trace(&self, message: &str) {
            self.0.lock().unwrap().push(message.to_owned());
        }
        fn avm_warning(&self, message: &str) {
            panic!("AVM warning: {message}");
        }
    }

    fn run(source: &[u8], behavior: LoadBehavior) -> u32 {
        let output = Arc::new(Mutex::new(Vec::new()));
        let player = PlayerBuilder::new()
            .with_movie(
                SwfMovie::from_data(source, "file:///MonsterBirthLookup.swf".into(), None, None)
                    .unwrap(),
            )
            .with_autoplay(true)
            .with_load_behavior(behavior)
            .with_log(Log(output.clone()))
            .build();
        for _ in 0..3 {
            player.lock().unwrap().run_frame();
        }
        let output = output.lock().unwrap();
        assert_eq!(output.len(), 2, "{output:?}");
        assert_eq!(output[1], "MONSTER_BIRTH_LOOKUP_OK");
        output[0]
            .strip_prefix("LOOKUP_BENCH_MS=")
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn birth_lookup_preserves_values_nested_enumeration_removal_and_rebuild() {
        let patched = patch_swf(FIXTURE);
        let mut abc = abc_with_lookup(FIXTURE);
        assert!(monster_birth::patch(&mut abc));
        assert_eq!(abc, abc_with_lookup(&patched));
        for behavior in [LoadBehavior::Blocking, LoadBehavior::Streaming] {
            run(FIXTURE, behavior);
            run(&patched, behavior);
        }
    }

    #[test]
    fn birth_lookup_is_idempotent_and_skips_unrecognized_bytecode() {
        let original = abc_with_lookup(FIXTURE);
        let mut patched = original.clone();
        assert!(monster_birth::patch(&mut patched));
        let once = patched.clone();
        assert!(!monster_birth::patch(&mut patched));
        assert_eq!(patched, once);
        // Find the body changed by the patch, without depending on compiler indices.
        let index = original
            .method_bodies
            .iter()
            .zip(&once.method_bodies)
            .position(|(a, b)| a.code != b.code)
            .unwrap();
        let mut unknown = original.clone();
        unknown.method_bodies[index].code.insert(0, 0x02); // unrecognized NOP
        let unchanged = unknown.clone();
        assert!(!monster_birth::patch(&mut unknown));
        assert_eq!(unknown, unchanged);
        let mut wrong_branch = original;
        let code = &mut wrong_branch.method_bodies[index].code;
        use swf::extensions::ReadSwfExt;
        let mut reader = AbcReader::new(code);
        let mut jump = None;
        while !reader.as_slice().is_empty() {
            let offset = code.len() - reader.as_slice().len();
            if matches!(reader.read_op().unwrap(), Op::Jump { .. }) {
                jump = Some(offset);
                break;
            }
        }
        code[jump.unwrap() + 1] ^= 1;
        let unchanged = wrong_branch.clone();
        assert!(!monster_birth::patch(&mut wrong_branch));
        assert_eq!(wrong_branch, unchanged);
    }

    #[test]
    fn birth_lookup_skips_a_changed_builder_or_unrelated_class() {
        let mut changed_builder = abc_with_lookup(FIXTURE);
        let method = changed_builder
            .instances
            .iter()
            .flat_map(|i| &i.traits)
            .find_map(|t| {
                if multiname_local_name(&changed_builder, t.name) != Some(b"buildBirthFromObjectVo")
                {
                    return None;
                }
                match t.kind {
                    TraitKind::Method { method, .. } => Some(method),
                    _ => None,
                }
            })
            .unwrap();
        let body = changed_builder.methods[method.0 as usize].body.unwrap();
        changed_builder.method_bodies[body.0 as usize].code = vec![0x47];
        let unchanged = changed_builder.clone();
        assert!(!monster_birth::patch(&mut changed_builder));
        assert_eq!(changed_builder, unchanged);

        let mut unrelated = abc_with_lookup(FIXTURE);
        let manager = unrelated
            .instances
            .iter()
            .position(|i| multiname_local_name(&unrelated, i.name) == Some(b"ManCreateManager"))
            .unwrap();
        let other = unrelated
            .instances
            .iter()
            .find(|i| multiname_local_name(&unrelated, i.name) == Some(b"MonsterBirthLookup"))
            .unwrap()
            .name;
        unrelated.instances[manager].name = other;
        let unchanged = unrelated.clone();
        assert!(!monster_birth::patch(&mut unrelated));
        assert_eq!(unrelated, unchanged);
    }

    #[test]
    #[ignore = "manual release benchmark; no wall-clock threshold in CI"]
    fn profile_birth_lookup() {
        let patched = patch_swf(FIXTURE);
        let mut before = Vec::new();
        let mut after = Vec::new();
        for round in 0..7 {
            if round % 2 == 0 {
                before.push(run(FIXTURE, LoadBehavior::Blocking));
                after.push(run(&patched, LoadBehavior::Blocking));
            } else {
                after.push(run(&patched, LoadBehavior::Blocking));
                before.push(run(FIXTURE, LoadBehavior::Blocking));
            }
        }
        before.sort_unstable();
        after.sort_unstable();
        eprintln!(
            "44-entry dictionary, 8 death checks, 40 dependency lookups each: original_median_ms={} patched_median_ms={} original={before:?} patched={after:?}",
            before[3], after[3]
        );
    }

    #[test]
    #[ignore = "optional read-only check against a locally cached game SWF"]
    fn verify_local_birth_lookup_patch() {
        let source =
            std::fs::read(std::env::var("ZM_PROBE_SWF").expect("set ZM_PROBE_SWF")).unwrap();
        let mut abc = abc_with_lookup(&source);
        let before = abc.clone();
        assert!(
            monster_birth::patch(&mut abc),
            "cached method no longer matches reviewed lookup"
        );
        let changed = before
            .method_bodies
            .iter()
            .zip(&abc.method_bodies)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(changed, 1);
        let mut encoded = Vec::new();
        AbcWriter::new(&mut encoded).write(abc.clone()).unwrap();
        let roundtrip = AbcReader::new(&encoded).read().unwrap();
        assert!(roundtrip.method_bodies == abc.method_bodies);
        let mut reencoded = Vec::new();
        AbcWriter::new(&mut reencoded).write(roundtrip).unwrap();
        // Real constant pools contain NaN, which is unequal to itself under
        // AbcFile's derived PartialEq. Compare the serialized bits instead.
        assert!(encoded == reencoded, "ABC encoding changed on roundtrip");
        // Other independent runtime patches may share the same ABC tag.
        text_log::patch(&mut abc);
        encoded.clear();
        AbcWriter::new(&mut encoded).write(abc).unwrap();
        let through_runtime = abc_with_lookup(&patch_swf(&source));
        let mut runtime_bytes = Vec::new();
        AbcWriter::new(&mut runtime_bytes)
            .write(through_runtime)
            .unwrap();
        assert!(
            runtime_bytes == encoded,
            "SWF pipeline did not apply the expected method patch"
        );
        eprintln!("Local game lookup recognized; exactly one method changed; ABC roundtrip OK");
    }
}

#[cfg(test)]
mod hidden_log_tests {
    use super::birth_lookup_tests::{abc_with_class, patch_swf};
    use super::*;
    use ruffle_core::{LoadBehavior, PlayerBuilder, backend::log::LogBackend, tag_utils::SwfMovie};
    use std::sync::{Arc, Mutex};
    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/HiddenTextLog.swf");
    struct Log(Arc<Mutex<Vec<String>>>);
    impl LogBackend for Log {
        fn avm_trace(&self, message: &str) {
            self.0.lock().unwrap().push(message.to_owned());
        }
        fn avm_warning(&self, message: &str) {
            panic!("AVM warning: {message}");
        }
    }
    fn run(source: &[u8], behavior: LoadBehavior) -> (u32, Vec<String>) {
        let output = Arc::new(Mutex::new(Vec::new()));
        let player = PlayerBuilder::new()
            .with_movie(
                SwfMovie::from_data(source, "file:///HiddenTextLog.swf".into(), None, None)
                    .unwrap(),
            )
            .with_autoplay(true)
            .with_load_behavior(behavior)
            .with_log(Log(output.clone()))
            .build();
        for _ in 0..3 {
            player.lock().unwrap().run_frame();
        }
        let mut output = output.lock().unwrap().clone();
        assert_eq!(
            output.last().map(String::as_str),
            Some("HIDDEN_TEXT_LOG_OK")
        );
        let i = output
            .iter()
            .position(|s| s.starts_with("HIDDEN_LOG_BENCH_MS="))
            .unwrap();
        let ms = output
            .remove(i)
            .strip_prefix("HIDDEN_LOG_BENCH_MS=")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(output.len(), 1070); // every original trace, including hidden/error/catch paths
        (ms, output)
    }
    #[test]
    fn hidden_log_preserves_traces_history_reveal_visible_updates_and_exceptions() {
        let patched = patch_swf(FIXTURE);
        let mut abc = abc_with_class(FIXTURE, b"SimpleTextLog");
        assert!(text_log::patch(&mut abc));
        assert_eq!(abc, abc_with_class(&patched, b"SimpleTextLog"));
        for behavior in [LoadBehavior::Blocking, LoadBehavior::Streaming] {
            assert_eq!(run(FIXTURE, behavior).1, run(&patched, behavior).1);
        }
    }
    #[test]
    fn hidden_log_patch_is_atomic_idempotent_and_rejects_unknown_ui() {
        let original = abc_with_class(FIXTURE, b"SimpleTextLog");
        let mut patched = original.clone();
        assert!(text_log::patch(&mut patched));
        let once = patched.clone();
        assert!(!text_log::patch(&mut patched));
        assert_eq!(patched, once);
        for method_name in [b"onMouse".as_slice(), b"__keyup", b"txt", b"log"] {
            let mut unknown = original.clone();
            let m = unknown
                .instances
                .iter()
                .flat_map(|i| &i.traits)
                .find_map(|t| {
                    if multiname_local_name(&unknown, t.name) != Some(method_name) {
                        return None;
                    }
                    match t.kind {
                        TraitKind::Method { method, .. } | TraitKind::Getter { method, .. } => {
                            Some(method)
                        }
                        _ => None,
                    }
                })
                .unwrap();
            let index = unknown.methods[m.0 as usize].body.unwrap().0 as usize;
            unknown.method_bodies[index].code = vec![0x47];
            let before = unknown.clone();
            assert!(!text_log::patch(&mut unknown));
            assert_eq!(unknown, before);
        }
    }
    #[test]
    #[ignore = "manual release benchmark; no timing threshold in CI"]
    fn profile_hidden_text_log() {
        let patched = patch_swf(FIXTURE);
        let mut before = Vec::new();
        let mut after = Vec::new();
        for round in 0..5 {
            if round % 2 == 0 {
                before.push(run(FIXTURE, LoadBehavior::Blocking).0);
                after.push(run(&patched, LoadBehavior::Blocking).0);
            } else {
                after.push(run(&patched, LoadBehavior::Blocking).0);
                before.push(run(FIXTURE, LoadBehavior::Blocking).0);
            }
        }
        before.sort_unstable();
        after.sort_unstable();
        eprintln!(
            "64 hidden log appends after 1000 lines: before_ms={before:?} after_ms={after:?}"
        );
    }
    #[test]
    #[ignore = "read-only structural check of a locally cached game"]
    fn verify_local_hidden_text_log_patch() {
        let source =
            std::fs::read(std::env::var("ZM_PROBE_SWF").expect("set ZM_PROBE_SWF")).unwrap();
        let mut abc = abc_with_class(&source, b"SimpleTextLog");
        let before = abc.clone();
        assert!(text_log::patch(&mut abc), "unrecognized game logger");
        assert_eq!(
            before
                .method_bodies
                .iter()
                .zip(&abc.method_bodies)
                .filter(|(a, b)| a != b)
                .count(),
            3
        );
        let mut encoded = Vec::new();
        AbcWriter::new(&mut encoded).write(abc).unwrap();
        let mut runtime = Vec::new();
        AbcWriter::new(&mut runtime)
            .write(abc_with_class(&patch_swf(&source), b"SimpleTextLog"))
            .unwrap();
        assert!(encoded == runtime, "pipeline differs from isolated patch");
        let decoded = AbcReader::new(&encoded).read().unwrap();
        let mut roundtrip = Vec::new();
        AbcWriter::new(&mut roundtrip).write(decoded).unwrap();
        assert!(encoded == roundtrip, "ABC roundtrip differs");
    }
}
