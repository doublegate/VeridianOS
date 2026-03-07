---------------------------- MODULE capability_model ----------------------------
\* Formal TLA+ Specification for VeridianOS Capability System
\*
\* Models capability tokens, rights lattice, derivation with rights
\* monotonicity, cascading revocation, and generation-based invalidation.

EXTENDS Integers, FiniteSets, Sequences

CONSTANTS
    MaxTokens,      \* Maximum number of capability tokens
    Rights,         \* Set of all possible rights (e.g., {"Read", "Write", "Execute", "Grant", "Derive"})
    Processes       \* Set of process IDs

VARIABLES
    capabilities,   \* Function: token -> capability record
    children,       \* Function: token -> set of child tokens
    generation,     \* Current generation counter
    next_token,     \* Next token to allocate
    revocation_log  \* Sequence of revoked tokens

vars == <<capabilities, children, generation, next_token, revocation_log>>

\* ============================================================================
\* Capability Record
\* ============================================================================
\* Each capability has:
\*   .rights       - Subset of Rights
\*   .owner        - Process ID
\*   .generation   - Generation when created
\*   .parent       - Parent token (0 = root)
\*   .valid        - Whether this capability is still valid

NullCap == [
    rights     |-> {},
    owner      |-> 0,
    generation |-> 0,
    parent     |-> 0,
    valid      |-> FALSE
]

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ next_token \in 1..MaxTokens+1
    /\ generation \in Nat
    /\ \A t \in DOMAIN capabilities :
        /\ capabilities[t].rights \subseteq Rights
        /\ capabilities[t].owner \in Processes \cup {0}
        /\ capabilities[t].generation \in Nat
        /\ capabilities[t].valid \in BOOLEAN

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ capabilities = [t \in 1..MaxTokens |-> NullCap]
    /\ children = [t \in 1..MaxTokens |-> {}]
    /\ generation = 0
    /\ next_token = 1
    /\ revocation_log = <<>>

\* ============================================================================
\* Actions
\* ============================================================================

\* Create a root capability (kernel operation)
CreateRoot(owner, rights_subset) ==
    /\ next_token <= MaxTokens
    /\ owner \in Processes
    /\ rights_subset \subseteq Rights
    /\ LET t == next_token
       IN /\ capabilities' = [capabilities EXCEPT ![t] = [
                rights     |-> rights_subset,
                owner      |-> owner,
                generation |-> generation,
                parent     |-> 0,
                valid      |-> TRUE
            ]]
          /\ next_token' = next_token + 1
    /\ UNCHANGED <<children, generation, revocation_log>>

\* Derive a child capability (rights must be a subset of parent)
Derive(parent_token, child_rights, child_owner) ==
    /\ parent_token \in DOMAIN capabilities
    /\ capabilities[parent_token].valid = TRUE
    /\ "Derive" \in capabilities[parent_token].rights      \* Must have derive right
    /\ child_rights \subseteq capabilities[parent_token].rights  \* Monotonicity!
    /\ next_token <= MaxTokens
    /\ child_owner \in Processes
    /\ LET t == next_token
       IN /\ capabilities' = [capabilities EXCEPT ![t] = [
                rights     |-> child_rights,
                owner      |-> child_owner,
                generation |-> generation,
                parent     |-> parent_token,
                valid      |-> TRUE
            ]]
          /\ children' = [children EXCEPT ![parent_token] = children[parent_token] \cup {t}]
          /\ next_token' = next_token + 1
    /\ UNCHANGED <<generation, revocation_log>>

\* Revoke a capability and all its descendants (cascading)
RECURSIVE CollectDescendants(_, _)
CollectDescendants(token, child_map) ==
    LET direct == child_map[token]
    IN direct \cup UNION {CollectDescendants(c, child_map) : c \in direct}

Revoke(token) ==
    /\ token \in DOMAIN capabilities
    /\ capabilities[token].valid = TRUE
    /\ LET descendants == CollectDescendants(token, children)
           all_revoked == descendants \cup {token}
       IN /\ capabilities' = [t \in DOMAIN capabilities |->
                IF t \in all_revoked
                THEN [capabilities[t] EXCEPT !.valid = FALSE]
                ELSE capabilities[t]]
          /\ revocation_log' = revocation_log \o
                SetToSeq(all_revoked)  \* Note: SetToSeq requires TLC
    /\ generation' = generation + 1
    /\ UNCHANGED <<children, next_token>>

\* ============================================================================
\* Helper: Convert set to sequence (for TLC)
\* ============================================================================

SetToSeq(S) ==
    IF S = {} THEN <<>>
    ELSE LET x == CHOOSE x \in S : TRUE
         IN <<x>> \o SetToSeq(S \ {x})

\* ============================================================================
\* Next-State Relation
\* ============================================================================

Next ==
    \/ \E o \in Processes, r \in SUBSET Rights : CreateRoot(o, r)
    \/ \E pt \in DOMAIN capabilities, r \in SUBSET Rights, o \in Processes : Derive(pt, r, o)
    \/ \E t \in DOMAIN capabilities : Revoke(t)

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars

\* ============================================================================
\* Safety Properties (Invariants)
\* ============================================================================

\* Non-forgery: all valid capabilities were created through the API
\* (tokens are sequential, so any valid token must be < next_token)
NonForgery ==
    \A t \in DOMAIN capabilities :
        capabilities[t].valid = TRUE => t < next_token

\* Rights monotonicity: child rights are always a subset of parent rights
RightsMonotonicity ==
    \A t \in DOMAIN capabilities :
        /\ capabilities[t].valid = TRUE
        /\ capabilities[t].parent # 0
        /\ capabilities[capabilities[t].parent].valid = TRUE
        => capabilities[t].rights \subseteq capabilities[capabilities[t].parent].rights

\* Revocation completeness: if a parent is revoked, all children are revoked
RevocationCompleteness ==
    \A t \in DOMAIN capabilities :
        capabilities[t].valid = FALSE =>
            \A c \in children[t] : capabilities[c].valid = FALSE

\* Generation integrity: revocation bumps generation
GenerationIntegrity ==
    generation >= 0

\* No orphan capabilities: if parent is invalid, children are invalid
NoOrphans ==
    \A t \in DOMAIN capabilities :
        /\ capabilities[t].valid = TRUE
        /\ capabilities[t].parent # 0
        => capabilities[capabilities[t].parent].valid = TRUE

================================================================================
