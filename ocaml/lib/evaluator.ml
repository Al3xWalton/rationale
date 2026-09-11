open Model
module String_map = Map.Make (String)
module String_set = Set.Make (String)

type traversal = {
  paths : Proof.path list;
  anchors : Proof.path list;
  reachable : String_set.t;
}

let protocol_error (request : kernel_request) code message =
  Error_response
    {
      protocol_version = 1;
      request_id = request.request_id;
      error = { code; message };
    }

let duplicate values =
  let sorted = List.sort String.compare values in
  let rec find = function
    | left :: (right :: _ as rest) ->
        if String.equal left right then Some left else find rest
    | _ -> None
  in
  find sorted

let node_map (nodes : evidence_node list) =
  List.fold_left
    (fun records (node : evidence_node) -> String_map.add node.id node records)
    String_map.empty nodes

let validate (request : kernel_request) =
  if request.protocol_version <> 1 then
    Error
      ( Unsupported_protocol,
        Printf.sprintf "protocol version %d is unsupported"
          request.protocol_version )
  else if List.length request.nodes > Rules.max_nodes then
    Error (Resource_limit, "evidence slice exceeds the node limit")
  else if List.length request.edges > Rules.max_edges then
    Error (Resource_limit, "evidence slice exceeds the edge limit")
  else if request.goal.anchor_kinds = [] then
    Error (Invalid_request, "proof goal has no anchor kinds")
  else if
    List.exists
      (fun kind -> not (Rules.valid_anchor_kind kind))
      request.goal.anchor_kinds
  then Error (Invalid_request, "proof goal contains a non-rationale anchor kind")
  else
    match
      duplicate (List.map (fun (node : evidence_node) -> node.id) request.nodes)
    with
    | Some id -> Error (Invalid_request, "duplicate node id: " ^ id)
    | None -> (
        match
          duplicate
            (List.map (fun (edge : evidence_edge) -> edge.id) request.edges)
        with
        | Some id -> Error (Invalid_request, "duplicate edge id: " ^ id)
        | None -> (
            let nodes = node_map request.nodes in
            if not (String_map.mem request.goal.target_id nodes) then
              Error
                ( Missing_record,
                  "proof target is absent from the evidence slice" )
            else
              match
                List.find_opt
                  (fun (edge : evidence_edge) ->
                    (not (String_map.mem edge.source_id nodes))
                    || not (String_map.mem edge.target_id nodes))
                  request.edges
              with
              | Some edge ->
                  Error
                    ( Missing_record,
                      "edge references a missing node: " ^ edge.id )
              | None -> (
                  match
                    List.find_opt
                      (fun (conflict : conflict) ->
                        (not (String_map.mem conflict.left_node_id nodes))
                        || not (String_map.mem conflict.right_node_id nodes))
                      request.conflicts
                  with
                  | Some conflict ->
                      Error
                        ( Missing_record,
                          "conflict references a missing node: "
                          ^ conflict.rule_id )
                  | None -> Ok nodes)))

let superseded_nodes (nodes : evidence_node String_map.t)
    (edges : evidence_edge list) =
  List.fold_left
    (fun superseded (edge : evidence_edge) ->
      if edge.status <> Current || edge.kind <> Supersedes then superseded
      else
        match String_map.find_opt edge.source_id nodes with
        | Some source when source.status = Current ->
            String_set.add edge.target_id superseded
        | Some _ | None -> superseded)
    String_set.empty edges

let active_node superseded (node : evidence_node) =
  node.status = Current && not (String_set.mem node.id superseded)

let active_edges (nodes : evidence_node String_map.t) superseded
    (edges : evidence_edge list) =
  List.filter
    (fun (edge : evidence_edge) ->
      Rules.is_traversable edge
      && Option.fold ~none:false ~some:(active_node superseded)
           (String_map.find_opt edge.source_id nodes)
      && Option.fold ~none:false ~some:(active_node superseded)
           (String_map.find_opt edge.target_id nodes))
    edges
  |> List.sort (fun (left : evidence_edge) (right : evidence_edge) ->
      match String.compare left.source_id right.source_id with
      | 0 -> (
          match String.compare left.target_id right.target_id with
          | 0 -> String.compare left.id right.id
          | comparison -> comparison)
      | comparison -> comparison)

let outgoing_map (edges : evidence_edge list) =
  List.fold_left
    (fun outgoing (edge : evidence_edge) ->
      let current =
        Option.value ~default:[] (String_map.find_opt edge.source_id outgoing)
      in
      String_map.add edge.source_id (edge :: current) outgoing)
    String_map.empty (List.rev edges)

let traverse (goal : proof_goal) (nodes : evidence_node String_map.t) superseded
    (edges : evidence_edge list) =
  let outgoing = outgoing_map edges in
  let queue = Queue.create () in
  Queue.add
    ({ node_ids = [ goal.target_id ]; edge_ids = [] } : Proof.path)
    queue;
  let rec loop explored paths anchors reachable =
    if Queue.is_empty queue then
      Ok { paths = List.rev paths; anchors; reachable }
    else if explored >= Rules.max_path_states then
      Error (Resource_limit, "proof traversal exceeds the path-state limit")
    else
      let path = Queue.take queue in
      let node_id = List.hd (List.rev path.node_ids) in
      let reachable = String_set.add node_id reachable in
      match String_map.find_opt node_id nodes with
      | None -> Error (Missing_record, "traversal reached an absent node")
      | Some node ->
          if not (active_node superseded node) then
            loop (explored + 1) (path :: paths) anchors reachable
          else
            let anchors =
              if Rules.is_anchor goal node then path :: anchors else anchors
            in
            let candidates =
              Option.value ~default:[] (String_map.find_opt node_id outgoing)
            in
            List.iter
              (fun (edge : evidence_edge) ->
                if not (List.mem edge.target_id path.node_ids) then
                  Queue.add
                    ({
                       node_ids = path.node_ids @ [ edge.target_id ];
                       edge_ids = path.edge_ids @ [ edge.id ];
                     }
                      : Proof.path)
                    queue)
              candidates;
            loop (explored + 1) (path :: paths) anchors reachable
  in
  loop 0 [] [] String_set.empty

let structured_conflicts (nodes : evidence_node String_map.t) superseded =
  let decisions =
    String_map.bindings nodes |> List.map snd
    |> List.filter (fun (node : evidence_node) ->
        active_node superseded node
        && node.kind = Decision
        && Option.is_some node.subject_id
        && Option.is_some node.outcome_id)
  in
  List.concat_map
    (fun (left : evidence_node) ->
      List.filter_map
        (fun (right : evidence_node) ->
          if
            String.compare left.id right.id < 0
            && left.subject_id = right.subject_id
            && left.outcome_id <> right.outcome_id
          then
            Some
              {
                left_node_id = left.id;
                right_node_id = right.id;
                rule_id = "conflict.subject_outcome";
              }
          else None)
        decisions)
    decisions

let material_conflicts reachable superseded (nodes : evidence_node String_map.t)
    (conflicts : conflict list) =
  conflicts
  |> List.filter (fun (conflict : conflict) ->
      String_set.mem conflict.left_node_id reachable
      && String_set.mem conflict.right_node_id reachable
      && Option.fold ~none:false ~some:(active_node superseded)
           (String_map.find_opt conflict.left_node_id nodes)
      && Option.fold ~none:false ~some:(active_node superseded)
           (String_map.find_opt conflict.right_node_id nodes))
  |> Proof.normalize_conflicts

let deepest_path (paths : Proof.path list) =
  List.sort
    (fun (left : Proof.path) (right : Proof.path) ->
      match
        Int.compare (List.length right.edge_ids) (List.length left.edge_ids)
      with
      | 0 -> List.compare String.compare left.node_ids right.node_ids
      | comparison -> comparison)
    paths
  |> List.hd

let evaluate (request : kernel_request) =
  match validate request with
  | Error (code, message) -> protocol_error request code message
  | Ok nodes -> (
      let superseded = superseded_nodes nodes request.edges in
      let edges = active_edges nodes superseded request.edges in
      match traverse request.goal nodes superseded edges with
      | Error (code, message) -> protocol_error request code message
      | Ok traversal ->
          let conflicts =
            request.conflicts @ structured_conflicts nodes superseded
            |> material_conflicts traversal.reachable superseded nodes
          in
          let proof =
            if conflicts <> [] then Proof.conflicted conflicts
            else if traversal.anchors <> [] then
              Proof.established traversal.anchors
            else
              let frontier = deepest_path traversal.paths in
              let frontier_id = List.hd (List.rev frontier.node_ids) in
              let node = String_map.find frontier_id nodes in
              Proof.incomplete frontier node
          in
          Proof_response
            {
              protocol_version = 1;
              request_id = request.request_id;
              snapshot_id = request.snapshot_id;
              proof;
            })
