module Model = Rationale_kernel.Model
module Evaluator = Rationale_kernel.Evaluator

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

let request () =
  let path = Filename.concat (fixture_root ()) "request-established.json" in
  match
    read_file path |> Yojson.Safe.from_string |> Model.kernel_request_of_yojson
  with
  | Ok request -> request
  | Error detail -> failwith detail

let verdict = function
  | Model.Proof_response { proof; _ } -> proof.verdict
  | Model.Error_response { error; _ } -> Alcotest.fail error.message

let with_goal (request : Model.kernel_request) anchor_kinds =
  { request with goal = { request.goal with anchor_kinds } }

let established_fixture () =
  let actual =
    request () |> Evaluator.evaluate |> Model.kernel_response_to_yojson
  in
  let expected =
    Filename.concat (fixture_root ()) "response-established.canonical.json"
    |> read_file |> String.trim
  in
  Alcotest.(check string)
    "canonical proof" expected
    (Yojson.Safe.to_string actual)

let incomplete_without_decision () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let edges =
    List.filter
      (fun edge -> edge.Model.kind <> Model.Documents)
      request.Model.edges
  in
  let response = Evaluator.evaluate { request with edges } in
  Alcotest.(check bool) "partial" true (verdict response = Model.Partial)

let absent_without_history () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let response = Evaluator.evaluate { request with edges = [] } in
  Alcotest.(check bool)
    "not established" true
    (verdict response = Model.Not_established)

let historical_anchor_does_not_establish () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let nodes =
    List.map
      (fun (node : Model.evidence_node) ->
        if node.Model.kind = Model.Decision then
          { node with status = Model.Historical }
        else node)
      request.Model.nodes
  in
  let response = Evaluator.evaluate { request with nodes } in
  Alcotest.(check bool)
    "historical decision excluded" true
    (verdict response <> Model.Established)

let conflicting_decisions () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let decision =
    List.find
      (fun (node : Model.evidence_node) -> node.Model.id = "decision-1")
      request.Model.nodes
  in
  let competing =
    {
      decision with
      id = "decision-2";
      outcome_id = Some "remote-token-validation";
    }
  in
  let documents =
    List.find
      (fun (edge : Model.evidence_edge) -> edge.Model.kind = Model.Documents)
      request.Model.edges
  in
  let competing_edge =
    { documents with id = "edge-documents-2"; target_id = "decision-2" }
  in
  let response =
    Evaluator.evaluate
      {
        request with
        nodes = request.nodes @ [ competing ];
        edges = request.edges @ [ competing_edge ];
      }
  in
  Alcotest.(check bool) "conflicted" true (verdict response = Model.Conflicted)

let supersession_resolves_conflict () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let decision =
    List.find
      (fun (node : Model.evidence_node) -> node.Model.id = "decision-1")
      request.Model.nodes
  in
  let competing =
    {
      decision with
      id = "decision-2";
      outcome_id = Some "remote-token-validation";
    }
  in
  let documents =
    List.find
      (fun (edge : Model.evidence_edge) -> edge.Model.kind = Model.Documents)
      request.Model.edges
  in
  let competing_edge =
    { documents with id = "edge-documents-2"; target_id = "decision-2" }
  in
  let supersedes =
    {
      documents with
      id = "edge-supersedes-1";
      kind = Model.Supersedes;
      source_id = "decision-2";
      target_id = "decision-1";
    }
  in
  let response =
    Evaluator.evaluate
      {
        request with
        nodes = request.nodes @ [ competing ];
        edges = request.edges @ [ competing_edge; supersedes ];
      }
  in
  Alcotest.(check bool) "established" true (verdict response = Model.Established)

let cycle_terminates () =
  let request = with_goal (request ()) [ Model.Decision ] in
  let template = List.hd request.Model.edges in
  let cycle =
    {
      template with
      id = "edge-cycle-1";
      kind = Model.Documents;
      source_id = "review-1";
      target_id = "change-1";
    }
  in
  let response =
    Evaluator.evaluate { request with edges = cycle :: request.edges }
  in
  Alcotest.(check bool) "established" true (verdict response = Model.Established)

let () =
  Alcotest.run "proof-kernel"
    [
      ( "verdicts",
        [
          Alcotest.test_case "established fixture" `Quick established_fixture;
          Alcotest.test_case "partial missing decision" `Quick
            incomplete_without_decision;
          Alcotest.test_case "absent history" `Quick absent_without_history;
          Alcotest.test_case "historical anchor" `Quick
            historical_anchor_does_not_establish;
          Alcotest.test_case "conflicting decisions" `Quick
            conflicting_decisions;
          Alcotest.test_case "supersession" `Quick
            supersession_resolves_conflict;
          Alcotest.test_case "cycle" `Quick cycle_terminates;
        ] );
    ]
