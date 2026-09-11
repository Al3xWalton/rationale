type node_kind =
  | Code_target
  | Change
  | Work_item
  | Review
  | Decision
  | Verification

type edge_kind =
  | Introduced_by
  | Changed_by
  | Included_in_pr
  | Resolves_issue
  | Documents
  | Supersedes
  | Verified_by

type record_status = Current | Historical
type source_kind = Git | Github | Document | Verification_source | Synthetic

type origin = {
  source_kind : source_kind;
  locator : string;
  revision : string option;
  observed_at : string option;
}

type evidence_node = {
  id : string;
  kind : node_kind;
  status : record_status;
  subject_id : string option;
  outcome_id : string option;
  origin : origin;
}

type evidence_edge = {
  id : string;
  kind : edge_kind;
  source_id : string;
  target_id : string;
  status : record_status;
  origin : origin;
}

type conflict = {
  left_node_id : string;
  right_node_id : string;
  rule_id : string;
}

type proof_goal = { target_id : string; anchor_kinds : node_kind list }

type kernel_request = {
  protocol_version : int;
  request_id : string;
  snapshot_id : string;
  goal : proof_goal;
  nodes : evidence_node list;
  edges : evidence_edge list;
  conflicts : conflict list;
}

type verdict = Established | Partial | Not_established | Conflicted

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

type proof_chain = {
  node_ids : string list;
  edge_ids : string list;
  rule_ids : string list;
}

type proof_result = {
  verdict : verdict;
  proof_chains : proof_chain list;
  gaps : gap list;
  conflicts : conflict list;
}

type error_code =
  | Unsupported_protocol
  | Invalid_request
  | Missing_record
  | Resource_limit
  | Internal

type protocol_error = { code : error_code; message : string }

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

let ( let* ) = Result.bind
let error context expected = Error (context ^ ": expected " ^ expected)

let object_ fields =
  `Assoc
    (List.sort (fun (left, _) (right, _) -> String.compare left right) fields)

let string = function `String value -> Ok value | _ -> error "value" "string"
let integer = function `Int value -> Ok value | _ -> error "value" "integer"

let list decode = function
  | `List values ->
      List.fold_left
        (fun result value ->
          let* decoded = result in
          let* item = decode value in
          Ok (item :: decoded))
        (Ok []) values
      |> Result.map List.rev
  | _ -> error "value" "array"

let fields context allowed = function
  | `Assoc values -> (
      let keys = List.map fst values in
      let sorted = List.sort String.compare keys in
      let rec duplicate = function
        | left :: (right :: _ as rest) ->
            if String.equal left right then Some left else duplicate rest
        | _ -> None
      in
      match duplicate sorted with
      | Some key -> Error (context ^ ": duplicate field `" ^ key ^ "`")
      | None -> (
          match
            List.find_opt (fun (key, _) -> not (List.mem key allowed)) values
          with
          | Some (key, _) -> Error (context ^ ": unknown field `" ^ key ^ "`")
          | None -> Ok values))
  | _ -> error context "object"

let required context name decode values =
  match List.assoc_opt name values with
  | None -> Error (context ^ ": missing field `" ^ name ^ "`")
  | Some value -> (
      match decode value with
      | Ok decoded -> Ok decoded
      | Error detail -> Error (context ^ "." ^ name ^ ": " ^ detail))

let optional context name decode values =
  match List.assoc_opt name values with
  | None -> Ok None
  | Some value -> (
      match decode value with
      | Ok decoded -> Ok (Some decoded)
      | Error detail -> Error (context ^ "." ^ name ^ ": " ^ detail))

let string_of_node_kind = function
  | Code_target -> "code_target"
  | Change -> "change"
  | Work_item -> "work_item"
  | Review -> "review"
  | Decision -> "decision"
  | Verification -> "verification"

let node_kind_of_yojson = function
  | `String "code_target" -> Ok Code_target
  | `String "change" -> Ok Change
  | `String "work_item" -> Ok Work_item
  | `String "review" -> Ok Review
  | `String "decision" -> Ok Decision
  | `String "verification" -> Ok Verification
  | _ -> error "node kind" "closed node-kind string"

let string_of_edge_kind = function
  | Introduced_by -> "introduced_by"
  | Changed_by -> "changed_by"
  | Included_in_pr -> "included_in_pr"
  | Resolves_issue -> "resolves_issue"
  | Documents -> "documents"
  | Supersedes -> "supersedes"
  | Verified_by -> "verified_by"

let edge_kind_of_yojson = function
  | `String "introduced_by" -> Ok Introduced_by
  | `String "changed_by" -> Ok Changed_by
  | `String "included_in_pr" -> Ok Included_in_pr
  | `String "resolves_issue" -> Ok Resolves_issue
  | `String "documents" -> Ok Documents
  | `String "supersedes" -> Ok Supersedes
  | `String "verified_by" -> Ok Verified_by
  | _ -> error "edge kind" "closed edge-kind string"

let string_of_record_status = function
  | Current -> "current"
  | Historical -> "historical"

let record_status_of_yojson = function
  | `String "current" -> Ok Current
  | `String "historical" -> Ok Historical
  | _ -> error "record status" "`current` or `historical`"

let string_of_source_kind = function
  | Git -> "git"
  | Github -> "github"
  | Document -> "document"
  | Verification_source -> "verification"
  | Synthetic -> "synthetic"

let source_kind_of_yojson = function
  | `String "git" -> Ok Git
  | `String "github" -> Ok Github
  | `String "document" -> Ok Document
  | `String "verification" -> Ok Verification_source
  | `String "synthetic" -> Ok Synthetic
  | _ -> error "source kind" "closed source-kind string"

let origin_to_yojson (origin : origin) =
  let optional name value =
    Option.fold ~none:[] ~some:(fun item -> [ (name, `String item) ]) value
  in
  object_
    ([
       ("locator", `String origin.locator);
       ("source_kind", `String (string_of_source_kind origin.source_kind));
     ]
    @ optional "observed_at" origin.observed_at
    @ optional "revision" origin.revision)

let origin_of_yojson json =
  let context = "origin" in
  let* values =
    fields context [ "locator"; "observed_at"; "revision"; "source_kind" ] json
  in
  let* source_kind =
    required context "source_kind" source_kind_of_yojson values
  in
  let* locator = required context "locator" string values in
  let* revision = optional context "revision" string values in
  let* observed_at = optional context "observed_at" string values in
  Ok { source_kind; locator; revision; observed_at }

let evidence_node_to_yojson (node : evidence_node) =
  let optional name value =
    Option.fold ~none:[] ~some:(fun item -> [ (name, `String item) ]) value
  in
  object_
    ([
       ("id", `String node.id);
       ("kind", `String (string_of_node_kind node.kind));
       ("origin", origin_to_yojson node.origin);
       ("status", `String (string_of_record_status node.status));
     ]
    @ optional "outcome_id" node.outcome_id
    @ optional "subject_id" node.subject_id)

let evidence_node_of_yojson json =
  let context = "node" in
  let* values =
    fields context
      [ "id"; "kind"; "origin"; "outcome_id"; "status"; "subject_id" ]
      json
  in
  let* id = required context "id" string values in
  let* kind = required context "kind" node_kind_of_yojson values in
  let* status = required context "status" record_status_of_yojson values in
  let* subject_id = optional context "subject_id" string values in
  let* outcome_id = optional context "outcome_id" string values in
  let* origin = required context "origin" origin_of_yojson values in
  Ok { id; kind; status; subject_id; outcome_id; origin }

let evidence_edge_to_yojson (edge : evidence_edge) =
  object_
    [
      ("id", `String edge.id);
      ("kind", `String (string_of_edge_kind edge.kind));
      ("origin", origin_to_yojson edge.origin);
      ("source_id", `String edge.source_id);
      ("status", `String (string_of_record_status edge.status));
      ("target_id", `String edge.target_id);
    ]

let evidence_edge_of_yojson json =
  let context = "edge" in
  let* values =
    fields context
      [ "id"; "kind"; "origin"; "source_id"; "status"; "target_id" ]
      json
  in
  let* id = required context "id" string values in
  let* kind = required context "kind" edge_kind_of_yojson values in
  let* source_id = required context "source_id" string values in
  let* target_id = required context "target_id" string values in
  let* status = required context "status" record_status_of_yojson values in
  let* origin = required context "origin" origin_of_yojson values in
  Ok { id; kind; source_id; target_id; status; origin }

let conflict_to_yojson (conflict : conflict) =
  object_
    [
      ("left_node_id", `String conflict.left_node_id);
      ("right_node_id", `String conflict.right_node_id);
      ("rule_id", `String conflict.rule_id);
    ]

let conflict_of_yojson json =
  let context = "conflict" in
  let* values =
    fields context [ "left_node_id"; "right_node_id"; "rule_id" ] json
  in
  let* left_node_id = required context "left_node_id" string values in
  let* right_node_id = required context "right_node_id" string values in
  let* rule_id = required context "rule_id" string values in
  Ok { left_node_id; right_node_id; rule_id }

let proof_goal_to_yojson (goal : proof_goal) =
  object_
    [
      ( "anchor_kinds",
        `List
          (List.map
             (fun kind -> `String (string_of_node_kind kind))
             goal.anchor_kinds) );
      ("target_id", `String goal.target_id);
    ]

let proof_goal_of_yojson json =
  let context = "goal" in
  let* values = fields context [ "anchor_kinds"; "target_id" ] json in
  let* target_id = required context "target_id" string values in
  let* anchor_kinds =
    required context "anchor_kinds" (list node_kind_of_yojson) values
  in
  Ok { target_id; anchor_kinds }

let kernel_request_to_yojson (request : kernel_request) =
  object_
    [
      ("conflicts", `List (List.map conflict_to_yojson request.conflicts));
      ("edges", `List (List.map evidence_edge_to_yojson request.edges));
      ("goal", proof_goal_to_yojson request.goal);
      ("nodes", `List (List.map evidence_node_to_yojson request.nodes));
      ("protocol_version", `Int request.protocol_version);
      ("request_id", `String request.request_id);
      ("snapshot_id", `String request.snapshot_id);
    ]

let kernel_request_of_yojson json =
  let context = "request" in
  let* values =
    fields context
      [
        "conflicts";
        "edges";
        "goal";
        "nodes";
        "protocol_version";
        "request_id";
        "snapshot_id";
      ]
      json
  in
  let* protocol_version = required context "protocol_version" integer values in
  let* request_id = required context "request_id" string values in
  let* snapshot_id = required context "snapshot_id" string values in
  let* goal = required context "goal" proof_goal_of_yojson values in
  let* nodes = required context "nodes" (list evidence_node_of_yojson) values in
  let* edges = required context "edges" (list evidence_edge_of_yojson) values in
  let* conflicts =
    required context "conflicts" (list conflict_of_yojson) values
  in
  Ok
    { protocol_version; request_id; snapshot_id; goal; nodes; edges; conflicts }

let string_of_verdict = function
  | Established -> "established"
  | Partial -> "partial"
  | Not_established -> "not_established"
  | Conflicted -> "conflicted"

let verdict_of_yojson = function
  | `String "established" -> Ok Established
  | `String "partial" -> Ok Partial
  | `String "not_established" -> Ok Not_established
  | `String "conflicted" -> Ok Conflicted
  | _ -> error "verdict" "closed verdict string"

let string_of_gap_code = function
  | Target_to_change -> "target_to_change"
  | Change_to_review -> "change_to_review"
  | Change_to_work_item -> "change_to_work_item"
  | Work_item_to_decision -> "work_item_to_decision"
  | Decision_to_verification -> "decision_to_verification"
  | No_current_anchor -> "no_current_anchor"

let gap_code_of_yojson = function
  | `String "target_to_change" -> Ok Target_to_change
  | `String "change_to_review" -> Ok Change_to_review
  | `String "change_to_work_item" -> Ok Change_to_work_item
  | `String "work_item_to_decision" -> Ok Work_item_to_decision
  | `String "decision_to_verification" -> Ok Decision_to_verification
  | `String "no_current_anchor" -> Ok No_current_anchor
  | _ -> error "gap code" "closed gap-code string"

let gap_to_yojson (gap : gap) =
  let from_node =
    Option.fold ~none:[]
      ~some:(fun id -> [ ("from_node_id", `String id) ])
      gap.from_node_id
  in
  let to_kind =
    Option.fold ~none:[]
      ~some:(fun kind -> [ ("to_kind", `String (string_of_node_kind kind)) ])
      gap.to_kind
  in
  object_
    ([ ("code", `String (string_of_gap_code gap.code)) ] @ from_node @ to_kind)

let gap_of_yojson json =
  let context = "gap" in
  let* values = fields context [ "code"; "from_node_id"; "to_kind" ] json in
  let* code = required context "code" gap_code_of_yojson values in
  let* from_node_id = optional context "from_node_id" string values in
  let* to_kind = optional context "to_kind" node_kind_of_yojson values in
  Ok { code; from_node_id; to_kind }

let proof_chain_to_yojson (chain : proof_chain) =
  let strings values = `List (List.map (fun value -> `String value) values) in
  object_
    [
      ("edge_ids", strings chain.edge_ids);
      ("node_ids", strings chain.node_ids);
      ("rule_ids", strings chain.rule_ids);
    ]

let proof_chain_of_yojson json =
  let context = "proof chain" in
  let* values = fields context [ "edge_ids"; "node_ids"; "rule_ids" ] json in
  let* node_ids = required context "node_ids" (list string) values in
  let* edge_ids = required context "edge_ids" (list string) values in
  let* rule_ids = required context "rule_ids" (list string) values in
  Ok { node_ids; edge_ids; rule_ids }

let proof_result_to_yojson (proof : proof_result) =
  object_
    [
      ("conflicts", `List (List.map conflict_to_yojson proof.conflicts));
      ("gaps", `List (List.map gap_to_yojson proof.gaps));
      ("proof_chains", `List (List.map proof_chain_to_yojson proof.proof_chains));
      ("verdict", `String (string_of_verdict proof.verdict));
    ]

let proof_result_of_yojson json =
  let context = "proof" in
  let* values =
    fields context [ "conflicts"; "gaps"; "proof_chains"; "verdict" ] json
  in
  let* verdict = required context "verdict" verdict_of_yojson values in
  let* proof_chains =
    required context "proof_chains" (list proof_chain_of_yojson) values
  in
  let* gaps = required context "gaps" (list gap_of_yojson) values in
  let* conflicts =
    required context "conflicts" (list conflict_of_yojson) values
  in
  Ok { verdict; proof_chains; gaps; conflicts }

let string_of_error_code = function
  | Unsupported_protocol -> "unsupported_protocol"
  | Invalid_request -> "invalid_request"
  | Missing_record -> "missing_record"
  | Resource_limit -> "resource_limit"
  | Internal -> "internal"

let error_code_of_yojson = function
  | `String "unsupported_protocol" -> Ok Unsupported_protocol
  | `String "invalid_request" -> Ok Invalid_request
  | `String "missing_record" -> Ok Missing_record
  | `String "resource_limit" -> Ok Resource_limit
  | `String "internal" -> Ok Internal
  | _ -> error "error code" "closed error-code string"

let protocol_error_to_yojson (protocol_error : protocol_error) =
  object_
    [
      ("code", `String (string_of_error_code protocol_error.code));
      ("message", `String protocol_error.message);
    ]

let protocol_error_of_yojson json =
  let context = "error" in
  let* values = fields context [ "code"; "message" ] json in
  let* code = required context "code" error_code_of_yojson values in
  let* message = required context "message" string values in
  Ok { code; message }

let kernel_response_to_yojson = function
  | Proof_response { protocol_version; request_id; snapshot_id; proof } ->
      object_
        [
          ("proof", proof_result_to_yojson proof);
          ("protocol_version", `Int protocol_version);
          ("request_id", `String request_id);
          ("snapshot_id", `String snapshot_id);
          ("status", `String "proof");
        ]
  | Error_response { protocol_version; request_id; error = protocol_error } ->
      object_
        [
          ("error", protocol_error_to_yojson protocol_error);
          ("protocol_version", `Int protocol_version);
          ("request_id", `String request_id);
          ("status", `String "error");
        ]

let proof_response_of_fields values =
  let context = "proof response" in
  let* protocol_version = required context "protocol_version" integer values in
  let* request_id = required context "request_id" string values in
  let* snapshot_id = required context "snapshot_id" string values in
  let* proof = required context "proof" proof_result_of_yojson values in
  Ok (Proof_response { protocol_version; request_id; snapshot_id; proof })

let error_response_of_fields values =
  let context = "error response" in
  let* protocol_version = required context "protocol_version" integer values in
  let* request_id = required context "request_id" string values in
  let* error = required context "error" protocol_error_of_yojson values in
  Ok (Error_response { protocol_version; request_id; error })

let kernel_response_of_yojson json =
  match json with
  | `Assoc values -> (
      match List.assoc_opt "status" values with
      | Some (`String "proof") ->
          let* values =
            fields "proof response"
              [
                "proof";
                "protocol_version";
                "request_id";
                "snapshot_id";
                "status";
              ]
              json
          in
          proof_response_of_fields values
      | Some (`String "error") ->
          let* values =
            fields "error response"
              [ "error"; "protocol_version"; "request_id"; "status" ]
              json
          in
          error_response_of_fields values
      | Some _ -> error "response.status" "`proof` or `error`"
      | None -> Error "response: missing field `status`")
  | _ -> error "response" "object"
