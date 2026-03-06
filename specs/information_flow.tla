--------------------------- MODULE information_flow ---------------------------
\* Formal TLA+ Specification for Non-Interference Between Security Domains
\*
\* Proves that information cannot flow from high-security domains to
\* low-security domains without explicit declassification through
\* the capability system.

EXTENDS Integers, FiniteSets

CONSTANTS
    Domains,            \* Set of security domains (e.g., {"Low", "Medium", "High"})
    DomainOrder,        \* Partial order on domains: set of <<dom1, dom2>> where dom1 <= dom2
    Processes,          \* Set of process IDs
    Channels            \* Set of IPC channel IDs

VARIABLES
    process_domain,     \* Function: process -> security domain
    channel_label,      \* Function: channel -> security domain (label)
    data_flow,          \* Set of <<source_domain, dest_domain>> observed flows
    declassified        \* Set of <<source_domain, dest_domain>> explicitly allowed flows

vars == <<process_domain, channel_label, data_flow, declassified>>

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ process_domain \in [Processes -> Domains]
    /\ channel_label \in [Channels -> Domains]
    /\ data_flow \subseteq (Domains \X Domains)
    /\ declassified \subseteq (Domains \X Domains)

\* ============================================================================
\* Helper: Domain Ordering
\* ============================================================================

\* d1 dominates d2 (d1 >= d2 in the lattice)
Dominates(d1, d2) == <<d2, d1>> \in DomainOrder

\* d1 and d2 are at the same level
SameLevel(d1, d2) == d1 = d2

\* Flow from d1 to d2 is allowed (upward or same level)
FlowAllowed(from_dom, to_dom) ==
    \/ Dominates(to_dom, from_dom)  \* to_dom >= from_dom (upward flow OK)
    \/ SameLevel(from_dom, to_dom)  \* Same level OK
    \/ <<from_dom, to_dom>> \in declassified  \* Explicitly declassified

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ process_domain \in [Processes -> Domains]
    /\ channel_label \in [Channels -> Domains]
    /\ data_flow = {}
    /\ declassified = {}

\* ============================================================================
\* Actions
\* ============================================================================

\* Process sends data on a channel (data flows from process domain to channel label)
SendOnChannel(proc, ch) ==
    /\ proc \in Processes
    /\ ch \in Channels
    /\ LET src == process_domain[proc]
           dst == channel_label[ch]
       IN /\ FlowAllowed(src, dst)  \* Only allow if flow is permitted
          /\ data_flow' = data_flow \cup {<<src, dst>>}
    /\ UNCHANGED <<process_domain, channel_label, declassified>>

\* Process receives data from a channel (data flows from channel label to process domain)
ReceiveFromChannel(proc, ch) ==
    /\ proc \in Processes
    /\ ch \in Channels
    /\ LET src == channel_label[ch]
           dst == process_domain[proc]
       IN /\ FlowAllowed(src, dst)  \* Only allow if flow is permitted
          /\ data_flow' = data_flow \cup {<<src, dst>>}
    /\ UNCHANGED <<process_domain, channel_label, declassified>>

\* Kernel explicitly declassifies a flow (requires capability)
Declassify(from_dom, to_dom) ==
    /\ from_dom \in Domains
    /\ to_dom \in Domains
    /\ from_dom # to_dom
    /\ declassified' = declassified \cup {<<from_dom, to_dom>>}
    /\ UNCHANGED <<process_domain, channel_label, data_flow>>

\* ============================================================================
\* Next-State Relation
\* ============================================================================

Next ==
    \/ \E p \in Processes, c \in Channels : SendOnChannel(p, c)
    \/ \E p \in Processes, c \in Channels : ReceiveFromChannel(p, c)
    \/ \E d1, d2 \in Domains : Declassify(d1, d2)

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars

\* ============================================================================
\* Safety Properties
\* ============================================================================

\* MAIN PROPERTY: Non-interference
\* No information flows from a higher domain to a lower domain
\* unless explicitly declassified
NonInterference ==
    \A <<src, dst>> \in data_flow :
        FlowAllowed(src, dst)

\* No downward flow without declassification
NoDownwardFlow ==
    \A <<src, dst>> \in data_flow :
        \/ Dominates(dst, src)           \* Upward flow
        \/ SameLevel(src, dst)           \* Same level
        \/ <<src, dst>> \in declassified \* Explicitly allowed

\* Reflexivity of domain ordering
DomainReflexive ==
    \A d \in Domains : <<d, d>> \in DomainOrder

\* Transitivity check: if data flowed A->B and B->C, then A->C must be allowed
TransitiveFlowSafety ==
    \A <<a, b>> \in data_flow :
        \A <<c, d>> \in data_flow :
            b = c => FlowAllowed(a, d)

================================================================================
