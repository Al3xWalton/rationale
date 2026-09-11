open Model

type path = { node_ids : string list; edge_ids : string list }

let compare_string_lists = List.compare String.compare

let compare_paths (left : path) (right : path) =
  match
    Int.compare (List.length left.edge_ids) (List.length right.edge_ids)
  with
  | 0 -> (
      match compare_string_lists left.edge_ids right.edge_ids with
      | 0 -> compare_string_lists left.node_ids right.node_ids
      | comparison -> comparison)
  | comparison -> comparison

let compare_conflicts (left : conflict) (right : conflict) =
  match String.compare left.left_node_id right.left_node_id with
  | 0 -> (
      match String.compare left.right_node_id right.right_node_id with
      | 0 -> String.compare left.rule_id right.rule_id
      | comparison -> comparison)
  | comparison -> comparison

let normalize_conflicts (conflicts : conflict list) =
  List.sort_uniq compare_conflicts conflicts

let rec is_prefix left right =
  match (left, right) with
  | [], _ -> true
  | _, [] -> false
  | left_head :: left_tail, right_head :: right_tail ->
      String.equal left_head right_head && is_prefix left_tail right_tail

let maximal_paths paths =
  List.filter
    (fun (candidate : path) ->
      not
        (List.exists
           (fun (other : path) ->
             List.length other.edge_ids > List.length candidate.edge_ids
             && is_prefix candidate.node_ids other.node_ids
             && is_prefix candidate.edge_ids other.edge_ids)
           paths))
    paths

let established (paths : path list) =
  let proof_chains =
    maximal_paths paths
    |> List.sort_uniq compare_paths
    |> List.map (fun path ->
        {
          node_ids = path.node_ids;
          edge_ids = path.edge_ids;
          rule_ids = [ "edge.current"; "path.admissible"; "anchor.current" ];
        })
  in
  { verdict = Established; proof_chains; gaps = []; conflicts = [] }

let incomplete (path : path) (node : evidence_node) =
  let verdict = if path.edge_ids = [] then Not_established else Partial in
  { verdict; proof_chains = []; gaps = [ Rules.gap_for node ]; conflicts = [] }

let conflicted (conflicts : conflict list) =
  {
    verdict = Conflicted;
    proof_chains = [];
    gaps = [];
    conflicts = normalize_conflicts conflicts;
  }
