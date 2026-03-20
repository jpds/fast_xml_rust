-module(fxml_stream_rust_test).
-compile(export_all).

%% Run all tests
all() ->
    Tests = [
        fun test_parse_element_simple/0,
        fun test_parse_element_nested/0,
        fun test_parse_element_empty/0,
        fun test_parse_element_error/0,
        fun test_stream_basic/0,
        fun test_stream_no_gen_server/0,
        fun test_stream_gen_server/0,
        fun test_stream_xmpp_realistic/0,
        fun test_stream_restart/0,
        fun test_stream_chunked/0,
        fun test_stream_max_size/0,
        fun test_stream_close/0,
        fun test_stream_change_callback_pid/0,
        fun test_stream_end/0,
        fun test_dtd_rejection/0,
        fun test_namespace_basic/0,
        fun test_namespace_redundant_xmlns/0,
        fun test_stream_multiple_stanzas/0
    ],
    Results = lists:map(fun(F) ->
        Name = element(2, erlang:fun_info(F, name)),
        %% Flush any leftover messages from prior tests
        flush_all(),
        try
            F(),
            io:format("  PASS: ~p~n", [Name]),
            pass
        catch
            C:R:ST ->
                io:format("  FAIL: ~p ~p:~p~n    ~p~n", [Name, C, R, hd(ST)]),
                fail
        end
    end, Tests),
    Passed = length([X || X <- Results, X == pass]),
    Failed = length([X || X <- Results, X == fail]),
    io:format("~n~p passed, ~p failed~n", [Passed, Failed]),
    case Failed of
        0 -> ok;
        _ -> error
    end.

%% --- parse_element tests ---

test_parse_element_simple() ->
    {xmlel, <<"msg">>, [{<<"to">>, <<"bob">>}], [{xmlcdata, <<"hello">>}]} =
        fxml_stream_rust:parse_element(<<"<msg to=\"bob\">hello</msg>">>),
    ok.

test_parse_element_nested() ->
    {xmlel, <<"a">>, [], [
        {xmlel, <<"b">>, [{<<"x">>, <<"1">>}], [
            {xmlcdata, <<"text">>}
        ]}
    ]} = fxml_stream_rust:parse_element(<<"<a><b x=\"1\">text</b></a>">>),
    ok.

test_parse_element_empty() ->
    {xmlel, <<"br">>, [], []} =
        fxml_stream_rust:parse_element(<<"<br/>">>),
    ok.

test_parse_element_error() ->
    {error, _} = fxml_stream_rust:parse_element(<<"not xml">>),
    ok.

%% --- streaming tests ---

test_stream_basic() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<root><child attr=\"val\">text</child></root>">>),
    {xmlstreamstart, <<"root">>, []} = receive_msg(),
    {xmlstreamelement, {xmlel, <<"child">>, [{<<"attr">>, <<"val">>}],
        [{xmlcdata, <<"text">>}]}} = receive_msg(),
    {xmlstreamend, <<"root">>} = receive_msg(),
    fxml_stream_rust:close(S1),
    ok.

test_stream_no_gen_server() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<root>">>),
    %% Should NOT be wrapped
    {xmlstreamstart, <<"root">>, []} = receive_msg(),
    fxml_stream_rust:close(S1),
    ok.

test_stream_gen_server() ->
    S0 = fxml_stream_rust:new(self(), infinity),
    S1 = fxml_stream_rust:parse(S0, <<"<root><child/></root>">>),
    {'$gen_event', {xmlstreamstart, <<"root">>, []}} = receive_msg(),
    {'$gen_event', {xmlstreamelement, {xmlel, <<"child">>, [], []}}} = receive_msg(),
    {'$gen_event', {xmlstreamend, <<"root">>}} = receive_msg(),
    fxml_stream_rust:close(S1),
    ok.

test_stream_xmpp_realistic() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    Stream = <<"<stream:stream xmlns='jabber:client' "
               "xmlns:stream='http://etherx.jabber.org/streams' "
               "to='example.com' version='1.0'>">>,
    S1 = fxml_stream_rust:parse(S0, Stream),
    {xmlstreamstart, <<"stream:stream">>, Attrs} = receive_msg(),
    true = lists:keymember(<<"xmlns">>, 1, Attrs),
    true = lists:keymember(<<"to">>, 1, Attrs),
    true = lists:keymember(<<"version">>, 1, Attrs),
    %% Send a message stanza
    Stanza = <<"<message to='alice@example.com' type='chat'>"
               "<body>Hello!</body>"
               "</message>">>,
    S2 = fxml_stream_rust:parse(S1, Stanza),
    {xmlstreamelement, {xmlel, <<"message">>, MsgAttrs, Children}} = receive_msg(),
    {<<"to">>, <<"alice@example.com">>} = lists:keyfind(<<"to">>, 1, MsgAttrs),
    {<<"type">>, <<"chat">>} = lists:keyfind(<<"type">>, 1, MsgAttrs),
    [{xmlel, <<"body">>, [], [{xmlcdata, <<"Hello!">>}]}] = Children,
    %% Send IQ stanza
    IQ = <<"<iq type='get' id='1'><query xmlns='jabber:iq:roster'/></iq>">>,
    S3 = fxml_stream_rust:parse(S2, IQ),
    {xmlstreamelement, {xmlel, <<"iq">>, _, _}} = receive_msg(),
    %% Close stream
    _S4 = fxml_stream_rust:parse(S3, <<"</stream:stream>">>),
    {xmlstreamend, <<"stream:stream">>} = receive_msg(),
    ok.

test_stream_restart() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<stream:stream>">>),
    {xmlstreamstart, <<"stream:stream">>, []} = receive_msg(),
    S2 = fxml_stream_rust:parse(S1, <<"<msg>first</msg>">>),
    {xmlstreamelement, {xmlel, <<"msg">>, [], [{xmlcdata, <<"first">>}]}} = receive_msg(),
    %% Simulate TLS negotiation: reset parser state
    S3 = fxml_stream_rust:reset(S2),
    %% New stream on same connection
    S4 = fxml_stream_rust:parse(S3, <<"<stream:stream>">>),
    {xmlstreamstart, <<"stream:stream">>, []} = receive_msg(),
    S5 = fxml_stream_rust:parse(S4, <<"<msg>second</msg>">>),
    {xmlstreamelement, {xmlel, <<"msg">>, [], [{xmlcdata, <<"second">>}]}} = receive_msg(),
    fxml_stream_rust:close(S5),
    ok.

test_stream_chunked() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    %% Send stream opening in chunks
    S1 = fxml_stream_rust:parse(S0, <<"<stream">>),
    no_msg = receive_or_timeout(),
    S2 = fxml_stream_rust:parse(S1, <<":stream>">>),
    {xmlstreamstart, <<"stream:stream">>, []} = receive_msg(),
    %% Send stanza in chunks
    S3 = fxml_stream_rust:parse(S2, <<"<msg to=">>),
    no_msg = receive_or_timeout(),
    S4 = fxml_stream_rust:parse(S3, <<"\"bob\">hel">>),
    no_msg = receive_or_timeout(),
    S5 = fxml_stream_rust:parse(S4, <<"lo</msg>">>),
    {xmlstreamelement, {xmlel, <<"msg">>, [{<<"to">>, <<"bob">>}],
        [{xmlcdata, <<"hello">>}]}} = receive_msg(),
    fxml_stream_rust:close(S5),
    ok.

test_stream_max_size() ->
    %% Max size of 50 bytes
    S0 = fxml_stream_rust:new(self(), 50, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<root>">>),
    {xmlstreamstart, <<"root">>, []} = receive_msg(),
    %% Send a stanza larger than 50 bytes total
    BigStanza = <<"<message><body>This is a very long message that exceeds the limit</body></message>">>,
    _S2 = fxml_stream_rust:parse(S1, BigStanza),
    {xmlstreamerror, <<"XML stanza is too big">>} = receive_msg(),
    ok.

test_stream_close() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    true = fxml_stream_rust:close(S0),
    ok.

test_stream_change_callback_pid() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    %% Spawn a helper to receive the message
    Parent = self(),
    Helper = spawn(fun() ->
        receive
            Msg -> Parent ! {helper_got, Msg}
        after 500 ->
            Parent ! {helper_got, timeout}
        end
    end),
    S1 = fxml_stream_rust:change_callback_pid(S0, Helper),
    _S2 = fxml_stream_rust:parse(S1, <<"<root>">>),
    %% We should NOT get xmlstream* directly (it went to Helper)
    no_xmlstream = receive_xmlstream_or_timeout(),
    %% Helper should have forwarded it
    {helper_got, {xmlstreamstart, <<"root">>, []}} = receive_msg(500),
    ok.

test_stream_end() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<stream></stream>">>),
    {xmlstreamstart, <<"stream">>, []} = receive_msg(),
    {xmlstreamend, <<"stream">>} = receive_msg(),
    fxml_stream_rust:close(S1),
    ok.

test_dtd_rejection() ->
    {error, _} = fxml_stream_rust:parse_element(<<"<!DOCTYPE foo><foo/>">>),
    ok.

test_namespace_basic() ->
    {xmlel, <<"query">>, Attrs, []} =
        fxml_stream_rust:parse_element(<<"<query xmlns='jabber:iq:roster'/>">>),
    {<<"xmlns">>, <<"jabber:iq:roster">>} = lists:keyfind(<<"xmlns">>, 1, Attrs),
    ok.

test_namespace_redundant_xmlns() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0,
        <<"<stream:stream xmlns='jabber:client' xmlns:stream='http://etherx.jabber.org/streams'>">>),
    {xmlstreamstart, <<"stream:stream">>, _} = receive_msg(),
    %% Message without explicit xmlns (common case) - should have no xmlns attr
    _S2 = fxml_stream_rust:parse(S1, <<"<message to='bob'><body>hi</body></message>">>),
    {xmlstreamelement, {xmlel, <<"message">>, MsgAttrs, _}} = receive_msg(),
    false = lists:keymember(<<"xmlns">>, 1, MsgAttrs),
    ok.

test_stream_multiple_stanzas() ->
    S0 = fxml_stream_rust:new(self(), infinity, [no_gen_server]),
    S1 = fxml_stream_rust:parse(S0, <<"<stream>">>),
    {xmlstreamstart, <<"stream">>, []} = receive_msg(),
    S2 = fxml_stream_rust:parse(S1, <<"<a/><b/><c/>">>),
    {xmlstreamelement, {xmlel, <<"a">>, [], []}} = receive_msg(),
    {xmlstreamelement, {xmlel, <<"b">>, [], []}} = receive_msg(),
    {xmlstreamelement, {xmlel, <<"c">>, [], []}} = receive_msg(),
    fxml_stream_rust:close(S2),
    ok.

%% --- helpers ---

receive_msg() ->
    receive_msg(200).

receive_msg(Timeout) ->
    receive
        Msg -> Msg
    after Timeout ->
        error(timeout)
    end.

receive_or_timeout() ->
    receive
        Msg -> Msg
    after 50 ->
        no_msg
    end.

receive_xmlstream_or_timeout() ->
    receive
        {xmlstreamstart, _, _} = Msg -> Msg;
        {xmlstreamelement, _} = Msg -> Msg;
        {xmlstreamend, _} = Msg -> Msg;
        {xmlstreamerror, _} = Msg -> Msg;
        {xmlstreamcdata, _} = Msg -> Msg
    after 50 ->
        no_xmlstream
    end.

flush_all() ->
    flush_all([]).
flush_all(Acc) ->
    receive
        Msg -> flush_all(Acc ++ [Msg])
    after 0 ->
        Acc
    end.
