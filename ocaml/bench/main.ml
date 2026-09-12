module Kernel = Rationale_kernel

let read_file path =
  let channel = open_in_bin path in
  Fun.protect
    ~finally:(fun () -> close_in channel)
    (fun () -> really_input_string channel (in_channel_length channel))

let positive_int value =
  match int_of_string_opt value with
  | Some value when value > 0 -> value
  | _ -> invalid_arg "iterations must be a positive integer"

let evaluate request iterations =
  for _ = 1 to 100 do
    ignore (Kernel.Evaluator.evaluate request)
  done;
  let started = Unix.gettimeofday () in
  for _ = 1 to iterations do
    ignore (Kernel.Evaluator.evaluate request)
  done;
  let elapsed = Unix.gettimeofday () -. started in
  let total_ms = elapsed *. 1_000.0 in
  `Assoc
    [
      ("iterations", `Int iterations);
      ("total_ms", `Float total_ms);
      ("per_evaluation_ms", `Float (total_ms /. float_of_int iterations));
    ]

let () =
  if Array.length Sys.argv <> 3 then
    invalid_arg "usage: rationale-kernel-bench REQUEST.json ITERATIONS";
  let request =
    Sys.argv.(1) |> read_file |> Yojson.Safe.from_string
    |> Kernel.Model.kernel_request_of_yojson
    |> function
    | Ok request -> request
    | Error message -> invalid_arg message
  in
  evaluate request (positive_int Sys.argv.(2))
  |> Yojson.Safe.to_string |> print_endline
