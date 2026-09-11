let test_protocol_version () =
  Alcotest.(check int) "protocol version" 1 Rationale_kernel.protocol_version

let () =
  Alcotest.run "rationale-kernel"
    [ ("bootstrap", [ Alcotest.test_case "protocol version" `Quick test_protocol_version ]) ]
