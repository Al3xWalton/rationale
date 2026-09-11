type read_error = Clean_eof | Truncated | Frame_too_large of int

let byte value shift = Char.chr ((value lsr shift) land 0xff)

let write_frame channel payload =
  let length = String.length payload in
  if length > 0x7fff_ffff then invalid_arg "worker frame exceeds uint32 range";
  output_char channel (byte length 24);
  output_char channel (byte length 16);
  output_char channel (byte length 8);
  output_char channel (byte length 0);
  output_string channel payload;
  flush channel

let read_header channel =
  match input_char channel with
  | exception End_of_file -> Error Clean_eof
  | first -> (
      try
        let second = input_char channel in
        let third = input_char channel in
        let fourth = input_char channel in
        let length =
          (Char.code first lsl 24)
          lor (Char.code second lsl 16)
          lor (Char.code third lsl 8)
          lor Char.code fourth
        in
        Ok length
      with End_of_file -> Error Truncated)

let read_frame ~max_length channel =
  match read_header channel with
  | Error error -> Error error
  | Ok length when length > max_length -> Error (Frame_too_large length)
  | Ok length -> (
      try Ok (really_input_string channel length)
      with End_of_file -> Error Truncated)
