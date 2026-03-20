use quick_xml::Reader;
use quick_xml::events::Event;
use rustler::{Encoder, Env, LocalPid, OwnedEnv};

use crate::atoms;
use crate::terms;

/// Represents a parsed XML element: {xmlel, Name, Attrs, Children}
#[derive(Debug, Clone)]
pub struct XmlEl {
    pub name: Vec<u8>,
    pub attrs: Vec<(Vec<u8>, Vec<u8>)>,
    pub children: Vec<XmlNode>,
}

/// A child node is either an element or character data
#[derive(Debug, Clone)]
pub enum XmlNode {
    Element(XmlEl),
    CData(Vec<u8>),
}

/// Send a term built in an OwnedEnv to a pid, using the caller's env
/// for the enif_send call (required on managed threads).
/// This mirrors fast_xml's C pattern: enif_send(caller_env, pid, send_env, term).
fn send_term_to_pid(caller_env: Env, pid: &LocalPid, build: impl FnOnce(Env) -> rustler::Term) {
    // We need to:
    // 1. Create a process-independent env to build terms in
    // 2. Build the term
    // 3. Send using enif_send with the caller env + msg env
    //
    // Rustler's Env::send always uses the env it's called on as both
    // caller and msg env. To use separate envs we call enif_send directly.
    let msg_env = OwnedEnv::new();
    msg_env.run(|menv| {
        let term = build(menv);
        let pid_c: *const rustler::sys::ErlNifPid = pid.as_c_arg() as *const _;
        // Safety: we're calling enif_send with a valid caller_env (from the NIF),
        // a valid pid, a valid msg_env (from OwnedEnv), and a term built in msg_env.
        let _rc = unsafe {
            rustler::sys::enif_send(
                caller_env.as_c_arg(),
                pid_c,
                menv.as_c_arg(),
                term.as_c_arg(),
            )
        };
    });
}

/// Streaming parser state held across NIF calls via ResourceArc
pub struct ParserState {
    pub callback_pid: LocalPid,
    pub max_size: usize,
    pub gen_server: bool,
    buffer: Vec<u8>,
    depth: usize,
    stack: Vec<XmlEl>,
    size: usize,
    closed: bool,
}

impl ParserState {
    pub fn new(pid: LocalPid, max_size: usize, gen_server: bool) -> Self {
        ParserState {
            callback_pid: pid,
            max_size,
            gen_server,
            buffer: Vec::new(),
            depth: 0,
            stack: Vec::new(),
            size: 0,
            closed: false,
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.depth = 0;
        self.stack.clear();
        self.size = 0;
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.buffer.clear();
        self.stack.clear();
    }

    pub fn feed(&mut self, env: Env, data: &[u8]) {
        if self.closed {
            return;
        }

        self.size += data.len();

        if self.size >= self.max_size {
            let size = self.size;
            self.send_error(env, b"XML stanza is too big");
            self.size = size;
            return;
        }

        self.buffer.extend_from_slice(data);
        self.parse_buffer(env);
    }

    fn parse_buffer(&mut self, env: Env) {
        let buf = std::mem::take(&mut self.buffer);
        // Use from_str for precise position tracking of consumed bytes.
        // SAFETY: We treat bytes as str here; quick-xml handles non-UTF8 gracefully.
        let buf_str = unsafe { std::str::from_utf8_unchecked(&buf) };
        let mut reader = Reader::from_str(buf_str);
        reader.config_mut().check_end_names = false;
        reader.config_mut().allow_unmatched_ends = true;
        reader.config_mut().expand_empty_elements = true;

        let mut consumed = 0usize;

        loop {
            match reader.read_event() {
                Ok(Event::Eof) => {
                    break;
                }
                Ok(Event::Start(ref e)) => {
                    consumed = reader.buffer_position() as usize;
                    self.depth += 1;

                    let name = e.name().as_ref().to_vec();
                    let attrs: Vec<(Vec<u8>, Vec<u8>)> = e
                        .attributes()
                        .filter_map(|a| a.ok())
                        .map(|a| (a.key.as_ref().to_vec(), a.value.to_vec()))
                        .collect();

                    if self.depth == 1 {
                        self.send_stream_start(env, &name, &attrs);
                    } else {
                        self.stack.push(XmlEl {
                            name,
                            attrs,
                            children: Vec::new(),
                        });
                    }
                }
                Ok(Event::End(ref e)) => {
                    consumed = reader.buffer_position() as usize;

                    if self.depth == 0 {
                        continue;
                    }

                    self.depth -= 1;

                    if self.depth == 0 {
                        let name = e.name().as_ref().to_vec();
                        self.send_stream_end(env, &name);
                    } else if self.depth == 1 {
                        if let Some(el) = self.stack.pop() {
                            self.send_stream_element(env, &el);
                            self.size = 0;
                        }
                    } else if let Some(el) = self.stack.pop()
                        && let Some(parent) = self.stack.last_mut()
                    {
                        parent.children.push(XmlNode::Element(el));
                    }
                }
                Ok(Event::Text(ref e)) => {
                    consumed = reader.buffer_position() as usize;
                    let text = e.as_ref().to_vec();

                    if self.depth == 1 {
                        self.send_stream_cdata(env, &text);
                    } else if self.depth > 1
                        && let Some(parent) = self.stack.last_mut()
                    {
                        if let Some(XmlNode::CData(existing)) = parent.children.last_mut() {
                            existing.extend_from_slice(&text);
                        } else {
                            parent.children.push(XmlNode::CData(text));
                        }
                    }
                }
                Ok(Event::CData(ref e)) => {
                    consumed = reader.buffer_position() as usize;
                    let text = e.to_vec();

                    if self.depth > 1
                        && let Some(parent) = self.stack.last_mut()
                    {
                        if let Some(XmlNode::CData(existing)) = parent.children.last_mut() {
                            existing.extend_from_slice(&text);
                        } else {
                            parent.children.push(XmlNode::CData(text));
                        }
                    }
                }
                Ok(Event::GeneralRef(_)) => {
                    self.send_error(env, b"Entity references are not allowed");
                    self.buffer.clear();
                    return;
                }
                Ok(Event::Empty(_)) => {
                    consumed = reader.buffer_position() as usize;
                }
                Ok(Event::Decl(_)) | Ok(Event::PI(_)) | Ok(Event::Comment(_)) => {
                    consumed = reader.buffer_position() as usize;
                }
                Ok(Event::DocType(_)) => {
                    self.send_error(env, b"DTDs are not allowed");
                    self.buffer.clear();
                    return;
                }
                Err(_) => {
                    break;
                }
            }
        }

        if consumed < buf.len() {
            self.buffer = buf[consumed..].to_vec();
        }
    }

    fn send_stream_start(&self, env: Env, name: &[u8], attrs: &[(Vec<u8>, Vec<u8>)]) {
        let gen_server = self.gen_server;
        let name = name.to_vec();
        let attrs = attrs.to_vec();
        send_term_to_pid(env, &self.callback_pid, |menv| {
            let name_term = terms::make_binary(menv, &name);
            let attrs_term = terms::make_attrs_list(menv, &attrs);
            let msg = (atoms::xmlstreamstart(), name_term, attrs_term).encode(menv);
            wrap_gen_server(menv, gen_server, msg)
        });
    }

    fn send_stream_end(&self, env: Env, name: &[u8]) {
        let gen_server = self.gen_server;
        let name = name.to_vec();
        send_term_to_pid(env, &self.callback_pid, |menv| {
            let name_term = terms::make_binary(menv, &name);
            let msg = (atoms::xmlstreamend(), name_term).encode(menv);
            wrap_gen_server(menv, gen_server, msg)
        });
    }

    fn send_stream_element(&self, env: Env, el: &XmlEl) {
        let gen_server = self.gen_server;
        let el = el.clone();
        send_term_to_pid(env, &self.callback_pid, |menv| {
            let el_term = terms::encode_xmlel(menv, &el);
            let msg = (atoms::xmlstreamelement(), el_term).encode(menv);
            wrap_gen_server(menv, gen_server, msg)
        });
    }

    fn send_stream_cdata(&self, env: Env, data: &[u8]) {
        let gen_server = self.gen_server;
        let data = data.to_vec();
        send_term_to_pid(env, &self.callback_pid, |menv| {
            let data_term = terms::make_binary(menv, &data);
            let msg = (atoms::xmlstreamcdata(), data_term).encode(menv);
            wrap_gen_server_all_state(menv, gen_server, msg)
        });
    }

    fn send_error(&mut self, env: Env, reason: &[u8]) {
        self.size = 0;
        let gen_server = self.gen_server;
        let reason = reason.to_vec();
        send_term_to_pid(env, &self.callback_pid, |menv| {
            let reason_term = terms::make_binary(menv, &reason);
            let msg = (atoms::xmlstreamerror(), reason_term).encode(menv);
            wrap_gen_server(menv, gen_server, msg)
        });
    }
}

fn wrap_gen_server<'a>(
    env: Env<'a>,
    gen_server: bool,
    msg: rustler::Term<'a>,
) -> rustler::Term<'a> {
    if gen_server {
        (atoms::gen_event(), msg).encode(env)
    } else {
        msg
    }
}

fn wrap_gen_server_all_state<'a>(
    env: Env<'a>,
    gen_server: bool,
    msg: rustler::Term<'a>,
) -> rustler::Term<'a> {
    if gen_server {
        (atoms::gen_all_state_event(), msg).encode(env)
    } else {
        msg
    }
}

/// Parse a complete XML element from a binary (not streaming).
pub fn parse_single_element(data: &[u8]) -> Result<XmlEl, String> {
    let mut reader = Reader::from_reader(data);
    reader.config_mut().check_end_names = true;
    reader.config_mut().expand_empty_elements = true;

    let mut buf = Vec::new();
    let mut stack: Vec<XmlEl> = Vec::new();

    stack.push(XmlEl {
        name: Vec::new(),
        attrs: Vec::new(),
        children: Vec::new(),
    });

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(Event::Start(ref e)) => {
                let name = e.name().as_ref().to_vec();
                let attrs: Vec<(Vec<u8>, Vec<u8>)> = e
                    .attributes()
                    .filter_map(|a| a.ok())
                    .map(|a| (a.key.as_ref().to_vec(), a.value.to_vec()))
                    .collect();
                stack.push(XmlEl {
                    name,
                    attrs,
                    children: Vec::new(),
                });
            }
            Ok(Event::End(_)) => {
                if let Some(el) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    parent.children.push(XmlNode::Element(el));
                }
            }
            Ok(Event::Text(ref e)) => {
                let text = e.as_ref().to_vec();
                if let Some(parent) = stack.last_mut() {
                    if let Some(XmlNode::CData(existing)) = parent.children.last_mut() {
                        existing.extend_from_slice(&text);
                    } else {
                        parent.children.push(XmlNode::CData(text));
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                let text = e.to_vec();
                if let Some(parent) = stack.last_mut() {
                    if let Some(XmlNode::CData(existing)) = parent.children.last_mut() {
                        existing.extend_from_slice(&text);
                    } else {
                        parent.children.push(XmlNode::CData(text));
                    }
                }
            }
            Ok(Event::GeneralRef(_)) => {
                return Err("Entity references are not allowed".to_string());
            }
            Ok(Event::DocType(_)) => {
                return Err("DTDs are not allowed".to_string());
            }
            Ok(_) => {}
            Err(e) => {
                return Err(format!("{}", e));
            }
        }
        buf.clear();
    }

    if let Some(root) = stack.pop() {
        for child in root.children {
            if let XmlNode::Element(el) = child {
                return Ok(el);
            }
        }
    }

    Err("not well-formed (invalid token)".to_string())
}
