use rustler::{Atom, Binary, Encoder, Env, LocalPid, NifResult, ResourceArc, Term};
use std::sync::Mutex;

mod stream;
mod terms;

pub mod atoms {
    rustler::atoms! {
        ok,
        error,
        true_ = "true",
        xmlstreamstart,
        xmlstreamelement,
        xmlstreamend,
        xmlstreamerror,
        xmlstreamcdata,
        xmlel,
        xmlcdata,
        gen_event = "$gen_event",
        gen_all_state_event = "$gen_all_state_event",
        no_gen_server,
        infinity,
    }
}

pub struct ParserResource {
    inner: Mutex<stream::ParserState>,
}

impl rustler::Resource for ParserResource {}

fn load(env: Env, _: Term) -> bool {
    env.register::<ParserResource>().is_ok()
}

#[rustler::nif(name = "new")]
fn new2(pid: LocalPid, max_size: Term) -> NifResult<ResourceArc<ParserResource>> {
    let max = parse_max_size(max_size)?;
    let state = stream::ParserState::new(pid, max, true);
    Ok(ResourceArc::new(ParserResource {
        inner: Mutex::new(state),
    }))
}

#[rustler::nif(name = "new")]
fn new3(
    pid: LocalPid,
    max_size: Term,
    options: Vec<Atom>,
) -> NifResult<ResourceArc<ParserResource>> {
    let max = parse_max_size(max_size)?;
    let gen_server = !options.contains(&atoms::no_gen_server());
    let state = stream::ParserState::new(pid, max, gen_server);
    Ok(ResourceArc::new(ParserResource {
        inner: Mutex::new(state),
    }))
}

fn parse_max_size(term: Term) -> NifResult<usize> {
    if let Ok(n) = term.decode::<u64>() {
        Ok(n as usize)
    } else if term.decode::<Atom>().ok() == Some(atoms::infinity()) {
        Ok(usize::MAX)
    } else {
        Err(rustler::Error::BadArg)
    }
}

#[rustler::nif]
fn parse<'a>(
    env: Env<'a>,
    resource: ResourceArc<ParserResource>,
    data: Binary,
) -> NifResult<Term<'a>> {
    let mut state = resource.inner.lock().map_err(|_| rustler::Error::BadArg)?;

    state.feed(env, data.as_slice());

    Ok(resource.clone().encode(env))
}

#[rustler::nif]
fn parse_element<'a>(env: Env<'a>, data: Binary) -> NifResult<Term<'a>> {
    match stream::parse_single_element(data.as_slice()) {
        Ok(el) => Ok(terms::encode_xmlel(env, &el)),
        Err(msg) => Ok(terms::encode_parse_error(env, &msg)),
    }
}

#[rustler::nif]
fn reset<'a>(env: Env<'a>, resource: ResourceArc<ParserResource>) -> NifResult<Term<'a>> {
    let mut state = resource.inner.lock().map_err(|_| rustler::Error::BadArg)?;

    state.reset();

    Ok(resource.clone().encode(env))
}

#[rustler::nif]
fn close(resource: ResourceArc<ParserResource>) -> Atom {
    if let Ok(mut state) = resource.inner.lock() {
        state.close();
    }
    atoms::true_()
}

#[rustler::nif]
fn change_callback_pid<'a>(
    env: Env<'a>,
    resource: ResourceArc<ParserResource>,
    new_pid: LocalPid,
) -> NifResult<Term<'a>> {
    let mut state = resource.inner.lock().map_err(|_| rustler::Error::BadArg)?;

    state.callback_pid = new_pid;

    Ok(resource.clone().encode(env))
}

rustler::init!("fxml_stream_rust", load = load);
