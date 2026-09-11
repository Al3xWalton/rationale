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
  | Ok request ->
      {
        request with
        goal = { request.goal with anchor_kinds = [ Model.Decision ] };
      }
  | Error detail -> failwith detail

let canonical (response : Model.kernel_response) =
  response |> Model.kernel_response_to_yojson |> Yojson.Safe.to_string

let verdict = function
  | Model.Proof_response { proof; _ } -> Some proof.verdict
  | Model.Error_response _ -> None

let input_order_is_irrelevant =
  QCheck.Test.make ~name:"input order does not change proof bytes" QCheck.bool
    (fun reverse ->
      let original = request () in
      let reordered =
        if reverse then
          {
            original with
            nodes = List.rev original.nodes;
            edges = List.rev original.edges;
          }
        else original
      in
      canonical (Evaluator.evaluate original)
      = canonical (Evaluator.evaluate reordered))

let removing_path_edge_downgrades =
  QCheck.Test.make ~name:"removing a decisive edge downgrades established"
    QCheck.(0 -- 3)
    (fun index ->
      let original = request () in
      let removed =
        original.edges |> List.filteri (fun current _ -> current <> index)
      in
      verdict (Evaluator.evaluate { original with edges = removed })
      <> Some Model.Established)

let run_property property () = QCheck.Test.check_exn property

let () =
  Alcotest.run "proof-properties"
    [
      ( "determinism",
        [
          Alcotest.test_case "input order" `Quick
            (run_property input_order_is_irrelevant);
          Alcotest.test_case "edge removal" `Quick
            (run_property removing_path_edge_downgrades);
        ] );
    ]
