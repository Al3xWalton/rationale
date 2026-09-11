module Kernel = Rationale_kernel

let max_frame_length = 8 * 1024 * 1024

let fail message =
  `Assoc [ ("kind", `String "failure"); ("message", `String message) ]

let exact_fields expected fields =
  let actual = List.map fst fields |> List.sort String.compare in
  let expected = List.sort String.compare expected in
  if actual = expected then Ok () else Error "worker message has invalid fields"

let field name fields =
  match List.assoc_opt name fields with
  | Some value -> Ok value
  | None -> Error ("worker message is missing " ^ name)

let int = function
  | `Int value -> Ok value
  | _ -> Error "protocol_version must be an integer"

let string = function
  | `String value -> Ok value
  | _ -> Error "kind must be a string"

let ready =
  `Assoc
    [
      ("kind", `String "ready");
      ("protocol_version", `Int Kernel.protocol_version);
    ]

let handle json =
  match json with
  | `Assoc fields -> (
      match Result.bind (field "kind" fields) string with
      | Error message -> fail message
      | Ok "hello" -> (
          match exact_fields [ "kind"; "protocol_version" ] fields with
          | Error message -> fail message
          | Ok () -> (
              match Result.bind (field "protocol_version" fields) int with
              | Ok version when version = Kernel.protocol_version -> ready
              | Ok _ -> fail "unsupported worker protocol version"
              | Error message -> fail message))
      | Ok "evaluate" -> (
          match exact_fields [ "kind"; "request" ] fields with
          | Error message -> fail message
          | Ok () -> (
              match field "request" fields with
              | Error message -> fail message
              | Ok request -> (
                  match Kernel.Model.kernel_request_of_yojson request with
                  | Error message -> fail message
                  | Ok request ->
                      `Assoc
                        [
                          ("kind", `String "result");
                          ( "response",
                            Kernel.Evaluator.evaluate request
                            |> Kernel.Model.kernel_response_to_yojson );
                        ])))
      | Ok _ -> fail "unknown worker message kind")
  | _ -> fail "worker message must be a JSON object"

let process payload =
  try payload |> Yojson.Safe.from_string |> handle
  with Yojson.Json_error message -> fail ("invalid worker JSON: " ^ message)

let rec loop () =
  match Framing.read_frame ~max_length:max_frame_length stdin with
  | Ok payload ->
      payload |> process |> Yojson.Safe.to_string |> Framing.write_frame stdout;
      loop ()
  | Error Clean_eof -> ()
  | Error Truncated -> prerr_endline "rationale worker: truncated input frame"
  | Error (Frame_too_large length) ->
      Printf.eprintf "rationale worker: frame length %d exceeds limit\n%!"
        length

let () =
  set_binary_mode_in stdin true;
  set_binary_mode_out stdout true;
  try loop ()
  with exn ->
    Printf.eprintf "rationale worker: fatal error: %s\n%!"
      (Printexc.to_string exn)
