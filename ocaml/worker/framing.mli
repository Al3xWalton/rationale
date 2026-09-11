type read_error = Clean_eof | Truncated | Frame_too_large of int

val read_frame : max_length:int -> in_channel -> (string, read_error) result
(** Read one four-byte big-endian length-prefixed frame. *)

val write_frame : out_channel -> string -> unit
(** Write and flush one four-byte big-endian length-prefixed frame. *)
