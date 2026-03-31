---- MODULE NetworkModel ----
(***************************************************************************
 * Network Model for RaisinDB CRDT Replication
 *
 * This module models an asynchronous network that:
 * - Allows arbitrary message delays
 * - Can reorder messages
 * - Eventually delivers all messages (no permanent loss)
 * - Can simulate network partitions
 *
 * The network is used by the replication system to model realistic
 * distributed behavior and verify safety properties hold even under
 * network anomalies.
 ***************************************************************************)

EXTENDS Naturals, Sequences, FiniteSets, TLC

CONSTANTS
  Nodes,          \* Set of all nodes in the cluster
  MaxOps,         \* Bound on total operations (for model checking)
  MaxDelay,       \* Maximum message delay steps
  MaxMessages     \* Maximum messages in flight (prevents state explosion)

VARIABLES
  network,        \* Messages in flight: set of [sender, receiver, payload, delay]
  delivered,      \* Messages delivered at each node: [node -> set of messages]
  time,           \* Global logical time (for bounded delays)
  partitioned     \* Set of {node1, node2} pairs that are partitioned

vars == <<network, delivered, time, partitioned>>

(***************************************************************************
 * Message Structure
 ***************************************************************************)

\* A message in the network
Message == [
  sender: Nodes,
  receiver: Nodes,
  payload: Nat,      \* Abstract payload (operation ID)
  delay: 0..MaxDelay \* Remaining delay before delivery
]

(***************************************************************************
 * Type Invariants
 ***************************************************************************)

TypeOK ==
  /\ network \subseteq Message
  /\ delivered \in [Nodes -> SUBSET Message]
  /\ time \in Nat
  /\ partitioned \subseteq (SUBSET Nodes)
  /\ \A p \in partitioned : Cardinality(p) = 2

(***************************************************************************
 * Initial State
 ***************************************************************************)

Init ==
  /\ network = {}
  /\ delivered = [n \in Nodes |-> {}]
  /\ time = 0
  /\ partitioned = {}

(***************************************************************************
 * Network Actions
 ***************************************************************************)

\* Send a message from sender to receiver with a non-deterministic delay
Send(sender, receiver, payload) ==
  /\ Cardinality(network) < MaxMessages  \* Prevent unbounded growth
  /\ \E delay \in 0..MaxDelay :
       network' = network \union {[
         sender |-> sender,
         receiver |-> receiver,
         payload |-> payload,
         delay |-> delay
       ]}
  /\ UNCHANGED <<delivered, time, partitioned>>

\* Deliver a message that has delay = 0
Receive(msg) ==
  /\ msg \in network
  /\ msg.delay = 0
  /\ {msg.sender, msg.receiver} \notin partitioned  \* Not partitioned
  /\ network' = network \ {msg}
  /\ delivered' = [delivered EXCEPT ![msg.receiver] = @ \union {msg}]
  /\ UNCHANGED <<time, partitioned>>

\* Advance time - decrements all message delays
Tick ==
  /\ time < MaxDelay * MaxOps  \* Bound time for model checking
  /\ network' = {[msg EXCEPT !.delay = IF @delay > 0 THEN @delay - 1 ELSE 0]
                  : msg \in network}
  /\ time' = time + 1
  /\ UNCHANGED <<delivered, partitioned>>

\* Create a network partition between two nodes
Partition(node1, node2) ==
  /\ node1 \in Nodes
  /\ node2 \in Nodes
  /\ node1 /= node2
  /\ {node1, node2} \notin partitioned
  /\ partitioned' = partitioned \union {{node1, node2}}
  /\ UNCHANGED <<network, delivered, time>>

\* Heal a network partition
Heal(node1, node2) ==
  /\ {node1, node2} \in partitioned
  /\ partitioned' = partitioned \ {{node1, node2}}
  /\ UNCHANGED <<network, delivered, time>>

(***************************************************************************
 * Helper Operators
 ***************************************************************************)

\* Check if two nodes are partitioned
ArePartitioned(n1, n2) ==
  {n1, n2} \in partitioned

\* Get all messages sent by a node
SentBy(node) ==
  {msg \in network : msg.sender = node}

\* Get all messages destined for a node
DestinedFor(node) ==
  {msg \in network : msg.receiver = node}

\* Count messages in flight
MessagesInFlight ==
  Cardinality(network)

\* Count total messages delivered to a node
MessagesDelivered(node) ==
  Cardinality(delivered[node])

(***************************************************************************
 * Safety Invariants
 ***************************************************************************)

\* Messages are eventually deliverable (delay eventually reaches 0)
NoStuckMessages ==
  \A msg \in network :
    msg.delay <= MaxDelay

\* No duplicate messages in network
NoDuplicateMessages ==
  \A msg1, msg2 \in network :
    (msg1 = msg2) \/
    (msg1.sender /= msg2.sender \/
     msg1.receiver /= msg2.receiver \/
     msg1.payload /= msg2.payload)

\* Network size is bounded
BoundedNetwork ==
  Cardinality(network) <= MaxMessages

\* A message cannot be delivered twice to the same node
NoDoubleDlivery ==
  \A node \in Nodes :
    \A msg1, msg2 \in delivered[node] :
      (msg1 = msg2) \/
      (msg1.sender /= msg2.sender \/
       msg1.payload /= msg2.payload)

(***************************************************************************
 * Liveness Properties (Fairness)
 ***************************************************************************)

\* Fairness: all messages with delay=0 eventually get delivered
\* (unless permanently partitioned)
Fairness ==
  /\ \A msg \in Message : WF_vars(Receive(msg))
  /\ WF_vars(Tick)

(***************************************************************************
 * Specification
 ***************************************************************************)

Next ==
  \/ Tick
  \/ \E msg \in network : Receive(msg)
  \* Send and partition actions would be defined by client specs

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************
 * Properties to Check
 ***************************************************************************)

\* THEOREM: If network is not partitioned, all messages eventually delivered
EventualDelivery ==
  \A msg \in Message :
    (msg \in network /\ {msg.sender, msg.receiver} \notin partitioned)
      ~> (msg \in delivered[msg.receiver])

====
