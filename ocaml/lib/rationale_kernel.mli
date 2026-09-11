val protocol_version : int
(** The protocol version implemented by this kernel. *)

module Model : module type of Model

module Rules : module type of Rules
(** Admissibility and proof-resource rules. *)

module Proof : module type of Proof
(** Canonical proof construction. *)

(** Pure proof evaluation. *)
module Evaluator : module type of Evaluator
(** Strict types and JSON codecs for the Rust-to-OCaml protocol. *)
