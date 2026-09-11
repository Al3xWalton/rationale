type path = { node_ids : string list; edge_ids : string list }
(** Traversal state used to construct a proof chain. *)

val established : path list -> Model.proof_result
(** Canonically order and build the maximal established proof chains. *)

val incomplete : path -> Model.evidence_node -> Model.proof_result
(** Build an incomplete result from the deepest deterministic frontier. *)

val conflicted : Model.conflict list -> Model.proof_result
(** Build a conflict result from material conflicts. *)

val normalize_conflicts : Model.conflict list -> Model.conflict list
(** Canonically sort and remove duplicate conflicts. *)
