val protocol_version : int
(** The protocol version implemented by this kernel. *)

module Model : module type of Model
(** Strict types and JSON codecs for the Rust-to-OCaml protocol. *)
