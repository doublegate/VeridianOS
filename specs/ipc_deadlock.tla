------------------------------ MODULE ipc_deadlock ------------------------------
\* Formal TLA+ Specification for IPC Deadlock Freedom
\*
\* Models the wait-for graph between processes and proves that
\* the system maintains deadlock freedom through cycle detection.

EXTENDS Integers, FiniteSets

CONSTANTS
    Processes       \* Set of process IDs

VARIABLES
    wait_for,       \* Function: process -> set of processes it's waiting for
    status,         \* Function: process -> "Running" | "Waiting" | "Blocked"
    held_resources  \* Function: process -> set of resources (channel IDs) held

vars == <<wait_for, status, held_resources>>

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ wait_for \in [Processes -> SUBSET Processes]
    /\ status \in [Processes -> {"Running", "Waiting", "Blocked"}]
    /\ \A p \in Processes : p \notin wait_for[p]  \* No self-edges

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ wait_for = [p \in Processes |-> {}]
    /\ status = [p \in Processes |-> "Running"]
    /\ held_resources = [p \in Processes |-> {}]

\* ============================================================================
\* Helper: Cycle Detection via Transitive Closure
\* ============================================================================

\* Reachable set from a process via wait_for edges
RECURSIVE Reachable(_, _, _)
Reachable(p, graph, visited) ==
    LET neighbors == graph[p] \ visited
    IN neighbors \cup
       UNION {Reachable(n, graph, visited \cup neighbors) : n \in neighbors}

\* Check if process p is in a cycle
InCycle(p) == p \in Reachable(p, wait_for, {})

\* Any cycle exists in the graph
HasCycle == \E p \in Processes : InCycle(p)

\* ============================================================================
\* Actions
\* ============================================================================

\* Process p starts waiting for process q (e.g., for a channel resource)
StartWaiting(p, q) ==
    /\ p \in Processes
    /\ q \in Processes
    /\ p # q
    /\ status[p] = "Running"
    \* Only allow if it would NOT create a cycle (deadlock prevention)
    /\ p \notin Reachable(q, wait_for, {})
    /\ wait_for' = [wait_for EXCEPT ![p] = wait_for[p] \cup {q}]
    /\ status' = [status EXCEPT ![p] = "Waiting"]
    /\ UNCHANGED held_resources

\* Process q completes, releasing process p from waiting
CompleteWait(p, q) ==
    /\ p \in Processes
    /\ q \in Processes
    /\ q \in wait_for[p]
    /\ wait_for' = [wait_for EXCEPT ![p] = wait_for[p] \ {q}]
    /\ status' = [status EXCEPT
            ![p] = IF wait_for[p] \ {q} = {} THEN "Running" ELSE "Waiting"]
    /\ UNCHANGED held_resources

\* Process p finishes and releases all wait dependencies
ProcessComplete(p) ==
    /\ p \in Processes
    /\ status[p] = "Running"
    \* Remove p from everyone's wait set
    /\ wait_for' = [q \in Processes |-> wait_for[q] \ {p}]
    /\ status' = [status EXCEPT ![p] = "Running"]
    /\ held_resources' = [held_resources EXCEPT ![p] = {}]

\* ============================================================================
\* Next-State Relation
\* ============================================================================

Next ==
    \/ \E p, q \in Processes : StartWaiting(p, q)
    \/ \E p, q \in Processes : CompleteWait(p, q)
    \/ \E p \in Processes : ProcessComplete(p)

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars

\* ============================================================================
\* Safety Properties
\* ============================================================================

\* MAIN PROPERTY: The system is always deadlock-free
DeadlockFreedom == ~HasCycle

\* No process waits for itself
NoSelfWait ==
    \A p \in Processes : p \notin wait_for[p]

\* Wait graph is finite (bounded by number of processes)
BoundedWaiting ==
    \A p \in Processes : Cardinality(wait_for[p]) <= Cardinality(Processes) - 1

\* If a process is running, it has no outstanding waits
RunningNotWaiting ==
    \A p \in Processes :
        status[p] = "Running" => wait_for[p] = {}
           \* Note: This is a simplification. In practice, a process may be
           \* running with pending waits if using async IPC.

================================================================================
