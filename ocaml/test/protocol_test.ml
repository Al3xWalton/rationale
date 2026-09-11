module Model = Rationale_kernel.Model

let fixture_root () =
  match Sys.getenv_opt "RATIONALE_FIXTURE_ROOT" with
  | Some root -> root
  | None ->
      failwith
        "RATIONALE_FIXTURE_ROOT must identify the shared protocol fixtures"

let read_file path =
  let channel = open_in_bin path in
  Fun.protect
    ~finally:(fun () -> close_in channel)
    (fun () -> really_input_string channel (in_channel_length channel))

let fixture name = Filename.concat (fixture_root ()) name

let check_request name () =
  let readable =
    read_file (fixture (name ^ ".json")) |> Yojson.Safe.from_string
  in
  let expected =
    read_file (fixture (name ^ ".canonical.json")) |> String.trim
  in
  match Model.kernel_request_of_yojson readable with
  | Error detail -> Alcotest.fail detail
  | Ok request ->
      let actual =
        Model.kernel_request_to_yojson request |> Yojson.Safe.to_string
      in
      Alcotest.(check string) "canonical request" expected actual

let check_response name () =
  let readable =
    read_file (fixture (name ^ ".json")) |> Yojson.Safe.from_string
  in
  let expected =
    read_file (fixture (name ^ ".canonical.json")) |> String.trim
  in
  match Model.kernel_response_of_yojson readable with
  | Error detail -> Alcotest.fail detail
  | Ok response ->
      let actual =
        Model.kernel_response_to_yojson response |> Yojson.Safe.to_string
      in
      Alcotest.(check string) "canonical response" expected actual

let reject_unknown_request () =
  let json =
    Yojson.Safe.from_string
      {|{"conflicts":[],"edges":[],"goal":{"anchor_kinds":["decision"],"target_id":"code-1"},"nodes":[],"protocol_version":1,"request_id":"request-1","snapshot_id":"snapshot-1","unexpected":true}|}
  in
  match Model.kernel_request_of_yojson json with
  | Error _ -> ()
  | Ok _ -> Alcotest.fail "unknown request field was accepted"

let reject_unknown_node_kind () =
  let json =
    Yojson.Safe.from_string
      {|{"conflicts":[],"edges":[],"goal":{"anchor_kinds":["decision"],"target_id":"code-1"},"nodes":[{"id":"code-1","kind":"future_record","origin":{"locator":"src/auth.rs:42","source_kind":"git"},"status":"current"}],"protocol_version":1,"request_id":"request-1","snapshot_id":"snapshot-1"}|}
  in
  match Model.kernel_request_of_yojson json with
  | Error _ -> ()
  | Ok _ -> Alcotest.fail "unknown node kind was accepted"

let () =
  Alcotest.run "protocol-v1"
    [
      ( "request",
        [
          Alcotest.test_case "established canonical round trip" `Quick
            (check_request "request-established");
          Alcotest.test_case "unknown field fails closed" `Quick
            reject_unknown_request;
          Alcotest.test_case "unknown enum fails closed" `Quick
            reject_unknown_node_kind;
        ] );
      ( "response",
        List.map
          (fun name -> Alcotest.test_case name `Quick (check_response name))
          [
            "response-established";
            "response-partial";
            "response-not-established";
            "response-conflicted";
            "response-error";
          ] );
    ]
