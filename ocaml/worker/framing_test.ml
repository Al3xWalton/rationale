let with_temp_file action =
  let path = Filename.temp_file "rationale-frame" ".bin" in
  Fun.protect ~finally:(fun () -> Sys.remove path) (fun () -> action path)

let round_trip () =
  with_temp_file (fun path ->
      let output = open_out_bin path in
      Framing.write_frame output "rationale";
      close_out output;
      let input = open_in_bin path in
      let actual = Framing.read_frame ~max_length:64 input in
      close_in input;
      Alcotest.(check (result string reject)) "payload" (Ok "rationale") actual)

let oversized () =
  with_temp_file (fun path ->
      let output = open_out_bin path in
      Framing.write_frame output "too large";
      close_out output;
      let input = open_in_bin path in
      let actual = Framing.read_frame ~max_length:4 input in
      close_in input;
      match actual with
      | Error (Frame_too_large 9) -> ()
      | Ok _ | Error _ ->
          Alcotest.fail "frame should exceed the configured bound")

let truncated () =
  with_temp_file (fun path ->
      let output = open_out_bin path in
      output_string output "\000\000\000\004abc";
      close_out output;
      let input = open_in_bin path in
      let actual = Framing.read_frame ~max_length:64 input in
      close_in input;
      match actual with
      | Error Truncated -> ()
      | Ok _ | Error _ -> Alcotest.fail "short payload should be truncated")

let () =
  Alcotest.run "worker-framing"
    [
      ( "frames",
        [
          Alcotest.test_case "round trip" `Quick round_trip;
          Alcotest.test_case "oversized" `Quick oversized;
          Alcotest.test_case "truncated" `Quick truncated;
        ] );
    ]
