val max_nodes : int
(** Maximum nodes accepted in one bounded evidence slice. *)

val max_edges : int
(** Maximum edges accepted in one bounded evidence slice. *)

val max_path_states : int
(** Maximum path states explored for one proof request. *)

val valid_anchor_kind : Model.node_kind -> bool
(** Whether a node kind may represent recorded intent. *)

val is_anchor : Model.proof_goal -> Model.evidence_node -> bool
(** Whether a current node satisfies a proof goal. *)

val is_traversable : Model.evidence_edge -> bool
(** Whether an edge may participate in a positive proof path. *)

val gap_for : Model.evidence_node -> Model.gap
(** The deterministic gap exposed at a terminal frontier node. *)
