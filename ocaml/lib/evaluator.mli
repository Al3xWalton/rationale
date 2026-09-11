val evaluate : Model.kernel_request -> Model.kernel_response
(** Evaluate one bounded evidence slice without performing I/O.

    The result is always either a deterministic proof response or a typed error;
    the evaluator never raises for a decoded request. *)
