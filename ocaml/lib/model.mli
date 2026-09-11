(** Kind of normalized evidence record. *)
type node_kind =
  | Code_target
  | Change
  | Work_item
  | Review
  | Decision
  | Verification

(** Explicit admissible relationship. *)
type edge_kind =
  | Introduced_by
  | Changed_by
  | Included_in_pr
  | Resolves_issue
  | Documents
  | Supersedes
  | Verified_by

(** Current or historical evidence status. *)
type record_status = Current | Historical

(** Origin system for evidence. *)
type source_kind = Git | Github | Document | Verification_source | Synthetic

type origin = {
  source_kind : source_kind;
  locator : string;
  revision : string option;
  observed_at : string option;
}
(** Stable source citation. *)

type evidence_node = {
  id : string;
  kind : node_kind;
  status : record_status;
  subject_id : string option;
  outcome_id : string option;
  origin : origin;
}
(** Normalized evidence node. *)

type evidence_edge = {
  id : string;
  kind : edge_kind;
  source_id : string;
  target_id : string;
  status : record_status;
  origin : origin;
}
(** Normalized admissible edge. *)

type conflict = {
  left_node_id : string;
  right_node_id : string;
  rule_id : string;
}
(** Explicit conflict metadata. *)

type proof_goal = { target_id : string; anchor_kinds : node_kind list }
(** Proof objective. *)

type kernel_request = {
  protocol_version : int;
  request_id : string;
  snapshot_id : string;
  goal : proof_goal;
  nodes : evidence_node list;
  edges : evidence_edge list;
  conflicts : conflict list;
}
(** One bounded request to the kernel. *)

(** Proof verdict. *)
type verdict = Established | Partial | Not_established | Conflicted

(** Typed missing relationship. *)
type gap_code =
  | Target_to_change
  | Change_to_review
  | Change_to_work_item
  | Work_item_to_decision
  | Decision_to_verification
  | No_current_anchor

type gap = {
  code : gap_code;
  from_node_id : string option;
  to_kind : node_kind option;
}
(** One gap in an incomplete proof. *)

type proof_chain = {
  node_ids : string list;
  edge_ids : string list;
  rule_ids : string list;
}
(** One machine-checkable proof route. *)

type proof_result = {
  verdict : verdict;
  proof_chains : proof_chain list;
  gaps : gap list;
  conflicts : conflict list;
}
(** Deterministic proof result. *)

(** Typed protocol failure code. *)
type error_code =
  | Unsupported_protocol
  | Invalid_request
  | Missing_record
  | Resource_limit
  | Internal

type protocol_error = { code : error_code; message : string }
(** Protocol failure without a verdict. *)

(** Proof or error response. *)
type kernel_response =
  | Proof_response of {
      protocol_version : int;
      request_id : string;
      snapshot_id : string;
      proof : proof_result;
    }
  | Error_response of {
      protocol_version : int;
      request_id : string;
      error : protocol_error;
    }

val kernel_request_to_yojson : kernel_request -> Yojson.Safe.t
val kernel_request_of_yojson : Yojson.Safe.t -> (kernel_request, string) result
val kernel_response_to_yojson : kernel_response -> Yojson.Safe.t

val kernel_response_of_yojson :
  Yojson.Safe.t -> (kernel_response, string) result
