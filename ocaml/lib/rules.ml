open Model

let max_nodes = 4_096
let max_edges = 16_384
let max_path_states = 65_536
let valid_anchor_kind = function Decision | Work_item -> true | _ -> false

let is_anchor (goal : proof_goal) (node : evidence_node) =
  node.status = Current && List.mem node.kind goal.anchor_kinds

let is_traversable (edge : evidence_edge) =
  edge.status = Current && edge.kind <> Supersedes

let gap_for (node : evidence_node) =
  match node.kind with
  | Code_target ->
      {
        code = Target_to_change;
        from_node_id = Some node.id;
        to_kind = Some Change;
      }
  | Change ->
      {
        code = Change_to_review;
        from_node_id = Some node.id;
        to_kind = Some Review;
      }
  | Review ->
      {
        code = Change_to_work_item;
        from_node_id = Some node.id;
        to_kind = Some Work_item;
      }
  | Work_item ->
      {
        code = Work_item_to_decision;
        from_node_id = Some node.id;
        to_kind = Some Decision;
      }
  | Decision ->
      {
        code = Decision_to_verification;
        from_node_id = Some node.id;
        to_kind = Some Verification;
      }
  | Verification ->
      { code = No_current_anchor; from_node_id = Some node.id; to_kind = None }
