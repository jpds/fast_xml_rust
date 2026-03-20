use rustler::{Encoder, Env, Term};

use crate::atoms;
use crate::stream::{XmlEl, XmlNode};

/// Create a binary term from a byte slice
pub fn make_binary<'a>(env: Env<'a>, data: &[u8]) -> Term<'a> {
    let mut bin = rustler::OwnedBinary::new(data.len()).expect("binary allocation");
    bin.as_mut_slice().copy_from_slice(data);
    bin.release(env).encode(env)
}

/// Create an attribute list: [{Key, Value}, ...]
pub fn make_attrs_list<'a>(env: Env<'a>, attrs: &[(Vec<u8>, Vec<u8>)]) -> Term<'a> {
    let pairs: Vec<Term<'a>> = attrs
        .iter()
        .map(|(k, v)| {
            let key = make_binary(env, k);
            let val = make_binary(env, v);
            (key, val).encode(env)
        })
        .collect();
    pairs.encode(env)
}

/// Encode an XmlEl as {xmlel, Name, Attrs, Children}
pub fn encode_xmlel<'a>(env: Env<'a>, el: &XmlEl) -> Term<'a> {
    let name = make_binary(env, &el.name);
    let attrs = make_attrs_list(env, &el.attrs);
    let children = encode_children(env, &el.children);
    (atoms::xmlel(), name, attrs, children).encode(env)
}

/// Encode a list of child nodes
fn encode_children<'a>(env: Env<'a>, children: &[XmlNode]) -> Term<'a> {
    let terms: Vec<Term<'a>> = children
        .iter()
        .map(|child| match child {
            XmlNode::Element(el) => encode_xmlel(env, el),
            XmlNode::CData(data) => {
                let bin = make_binary(env, data);
                (atoms::xmlcdata(), bin).encode(env)
            }
        })
        .collect();
    terms.encode(env)
}

/// Encode a parse error as {error, Reason}
pub fn encode_parse_error<'a>(env: Env<'a>, msg: &str) -> Term<'a> {
    let reason = make_binary(env, msg.as_bytes());
    (atoms::error(), reason).encode(env)
}
