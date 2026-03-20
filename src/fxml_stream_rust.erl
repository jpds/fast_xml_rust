-module(fxml_stream_rust).
-export([new/2, new/3, parse/2, parse_element/1,
         reset/1, close/1, change_callback_pid/2]).
-on_load(init/0).

init() ->
  SOPath = filename:join([code:priv_dir(fast_xml_rust), "crates", "fxml_stream_rust", "fxml_stream_rust"]),
  erlang:load_nif(SOPath, 0).

new(_Pid, _MaxSize)         -> erlang:nif_error(nif_not_loaded).
new(_Pid, _MaxSize, _Opts)  -> erlang:nif_error(nif_not_loaded).
parse(_State, _Data)        -> erlang:nif_error(nif_not_loaded).
parse_element(_Data)        -> erlang:nif_error(nif_not_loaded).
reset(_State)               -> erlang:nif_error(nif_not_loaded).
close(_State)               -> erlang:nif_error(nif_not_loaded).
change_callback_pid(_S, _P) -> erlang:nif_error(nif_not_loaded).
