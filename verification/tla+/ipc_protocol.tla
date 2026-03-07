------------------------------ MODULE ipc_protocol ------------------------------
\* Formal TLA+ Specification for VeridianOS IPC Protocol
\*
\* Models channel state machines, FIFO ordering, capacity bounds,
\* message conservation, and capability enforcement.

EXTENDS Integers, Sequences, FiniteSets

CONSTANTS
    MaxChannels,        \* Maximum number of channels
    MaxCapacity,        \* Maximum channel buffer capacity
    Processes,          \* Set of process IDs
    MaxMessages         \* Maximum messages to model (bound for model checking)

VARIABLES
    channels,           \* Function: channel_id -> channel state record
    next_seq,           \* Next global sequence number
    total_sent,         \* Total messages sent across all channels
    total_received      \* Total messages received across all channels

vars == <<channels, next_seq, total_sent, total_received>>

\* ============================================================================
\* Channel State Record
\* ============================================================================
\* Each channel has:
\*   .buffer    - Sequence of messages (FIFO queue)
\*   .capacity  - Maximum buffer size
\*   .senders   - Set of processes allowed to send
\*   .receivers - Set of processes allowed to receive
\*   .sent      - Number of messages sent on this channel
\*   .received  - Number of messages received on this channel

NullChannel == [
    buffer    |-> <<>>,
    capacity  |-> 0,
    senders   |-> {},
    receivers |-> {},
    sent      |-> 0,
    received  |-> 0
]

\* ============================================================================
\* Type Invariant
\* ============================================================================

TypeOK ==
    /\ next_seq \in Nat
    /\ total_sent \in Nat
    /\ total_received \in Nat
    /\ \A ch \in DOMAIN channels :
        /\ channels[ch].capacity \in 1..MaxCapacity
        /\ channels[ch].senders \subseteq Processes
        /\ channels[ch].receivers \subseteq Processes
        /\ channels[ch].sent \in Nat
        /\ channels[ch].received \in Nat
        /\ Len(channels[ch].buffer) <= channels[ch].capacity

\* ============================================================================
\* Initial State
\* ============================================================================

Init ==
    /\ channels = [ch \in 1..MaxChannels |-> [
            buffer    |-> <<>>,
            capacity  |-> MaxCapacity,
            senders   |-> Processes,
            receivers |-> Processes,
            sent      |-> 0,
            received  |-> 0
       ]]
    /\ next_seq = 0
    /\ total_sent = 0
    /\ total_received = 0

\* ============================================================================
\* Actions
\* ============================================================================

\* Send a message on channel ch from process p
Send(ch, p, payload) ==
    /\ ch \in DOMAIN channels
    /\ p \in channels[ch].senders                          \* Capability check
    /\ Len(channels[ch].buffer) < channels[ch].capacity    \* Capacity check
    /\ total_sent < MaxMessages                            \* Model bound
    /\ LET msg == [seq |-> next_seq, data |-> payload, sender |-> p, channel |-> ch]
       IN channels' = [channels EXCEPT
            ![ch].buffer = Append(channels[ch].buffer, msg),
            ![ch].sent = channels[ch].sent + 1]
    /\ next_seq' = next_seq + 1
    /\ total_sent' = total_sent + 1
    /\ UNCHANGED total_received

\* Receive a message from channel ch by process p
Receive(ch, p) ==
    /\ ch \in DOMAIN channels
    /\ p \in channels[ch].receivers                        \* Capability check
    /\ Len(channels[ch].buffer) > 0                        \* Non-empty check
    /\ LET msg == Head(channels[ch].buffer)
       IN channels' = [channels EXCEPT
            ![ch].buffer = Tail(channels[ch].buffer),
            ![ch].received = channels[ch].received + 1]
    /\ total_received' = total_received + 1
    /\ UNCHANGED <<next_seq, total_sent>>

\* ============================================================================
\* Next-State Relation
\* ============================================================================

Next ==
    \E ch \in DOMAIN channels, p \in Processes :
        \/ \E payload \in 0..9 : Send(ch, p, payload)
        \/ Receive(ch, p)

\* ============================================================================
\* Specification
\* ============================================================================

Spec == Init /\ [][Next]_vars /\ WF_vars(Next)

\* ============================================================================
\* Safety Properties (Invariants)
\* ============================================================================

\* FIFO ordering: messages in each buffer are in sequence order
FifoInvariant ==
    \A ch \in DOMAIN channels :
        \A i \in 1..Len(channels[ch].buffer)-1 :
            channels[ch].buffer[i].seq < channels[ch].buffer[i+1].seq

\* Capacity bound: no buffer exceeds its capacity
CapacityBound ==
    \A ch \in DOMAIN channels :
        Len(channels[ch].buffer) <= channels[ch].capacity

\* Message conservation: sent - received = pending
MessageConservation ==
    \A ch \in DOMAIN channels :
        channels[ch].sent - channels[ch].received = Len(channels[ch].buffer)

\* Global conservation
GlobalConservation ==
    total_sent >= total_received

\* Channel isolation: messages in channel ch have channel field = ch
ChannelIsolation ==
    \A ch \in DOMAIN channels :
        \A i \in 1..Len(channels[ch].buffer) :
            channels[ch].buffer[i].channel = ch

\* No unauthorized send: all messages have authorized senders
AuthorizedSenders ==
    \A ch \in DOMAIN channels :
        \A i \in 1..Len(channels[ch].buffer) :
            channels[ch].buffer[i].sender \in channels[ch].senders

================================================================================
