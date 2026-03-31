# IoT Dashboard: Real-Time Sensor Monitoring

Build a real-time IoT monitoring dashboard that handles sensor data streams, visualizes metrics, and uses branches for predictive analysis.

:::info What You'll Learn
- Model IoT devices and sensor readings
- Handle high-frequency time-series data
- Use WebSocket for real-time updates
- Create analysis branches for predictions without affecting production data
:::

## Prerequisites

- Completed the [Quickstart Tutorial](/docs/tutorials/quickstart)
- RaisinDB running locally
- Basic understanding of WebSocket (optional)

## What We're Building

A hospital equipment monitoring system that:
1. Tracks medical devices (ventilators, monitors, pumps)
2. Ingests sensor readings (temperature, pressure, flow rates)
3. Provides real-time dashboards via WebSocket
4. Uses branches to run "what-if" analysis for maintenance predictions

---

## Step 1: Model the Domain

<!--
OUTLINE FOR AUTHOR:

Create three NodeTypes:

**Device NodeType:**
- device_id (string, unique)
- device_type (enum: ventilator, monitor, pump, sensor)
- location (string - room/ward)
- status (enum: active, maintenance, offline)
- installed_date (datetime)
- last_maintenance (datetime)

**SensorReading NodeType:**
- device_id (reference to Device)
- timestamp (datetime, indexed)
- metric_type (enum: temperature, pressure, flow_rate, heart_rate, oxygen)
- value (float)
- unit (string)

**Alert NodeType:**
- device_id (reference)
- severity (enum: info, warning, critical)
- message (string)
- acknowledged (boolean)
- created_at (datetime)

Tips:
- Explain indexing strategy for time-series queries
- Mention how RaisinDB handles high-write workloads
-->

```bash
# TODO: Add NodeType creation commands
```

---

## Step 2: Seed Initial Data

<!--
OUTLINE FOR AUTHOR:
- Create 5-10 sample devices across different rooms
- Explain the data structure
- Show how to verify devices were created
-->

```bash
# TODO: Add device creation commands
```

---

## Step 3: Simulate Sensor Data

<!--
OUTLINE FOR AUTHOR:
- Create a simple script (bash loop or curl) to insert readings
- Insert readings every few seconds
- Show different metric types
- Include some readings that trigger thresholds (for alerts later)
-->

```bash
# TODO: Add sensor data simulation script
```

---

## Step 4: Query Time-Series Data

<!--
OUTLINE FOR AUTHOR:
- Query readings for a specific device
- Filter by time range (last hour, last 24h)
- Aggregate queries (avg, min, max per hour)
- Show how to join Device and SensorReading
-->

```sql
-- TODO: Add time-series query examples
```

---

## Step 5: Real-Time Updates with WebSocket

<!--
OUTLINE FOR AUTHOR:
- Connect to WebSocket endpoint
- Subscribe to a specific device's readings
- Subscribe to all critical alerts
- Show the message format
- Explain filtering/subscription options

Include a simple HTML/JS snippet they can open in browser:
<script>
  const ws = new WebSocket('ws://localhost:PORT/...');
  ws.onmessage = (event) => console.log(JSON.parse(event.data));
</script>
-->

```javascript
// TODO: Add WebSocket connection example
```

---

## Step 6: Analysis Branches for Predictions

<!--
OUTLINE FOR AUTHOR:

This is the key differentiator - show the power of branches for analytics:

1. Create a branch "maintenance-analysis"
2. On the branch, simulate accelerated wear:
   - Insert hypothetical future readings with degraded values
   - Model a failure scenario
3. Query the branch to find "when would device X fail?"
4. Compare branch data vs main data
5. Discard the branch (it was just for analysis)

Real-world use case:
"What if we delay maintenance by 2 weeks? At current degradation rate, when would we see critical failures?"

This shows:
- Branch isolation (main data untouched)
- Time-travel/what-if analysis
- Safe experimentation
-->

```bash
# TODO: Add branch-based analysis examples
```

---

## Step 7: Create Alert Rules (Optional Extension)

<!--
OUTLINE FOR AUTHOR:
- Show how to query for threshold violations
- Create Alert documents when thresholds exceeded
- This could be done via external trigger or future RaisinDB functions
- Keep it simple - just the concept
-->

```bash
# TODO: Add alert creation examples
```

---

## Architecture Recap

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  IoT Devices    │────▶│    RaisinDB     │────▶│   Dashboard     │
│  (Sensors)      │     │                 │     │   (WebSocket)   │
└─────────────────┘     │  ┌───────────┐  │     └─────────────────┘
                        │  │   main    │  │
                        │  └───────────┘  │
                        │        │        │
                        │  ┌─────▼─────┐  │
                        │  │ analysis  │  │     ← Branch for predictions
                        │  └───────────┘  │
                        └─────────────────┘
```

---

## What's Next?

You've learned how to:
- Model IoT devices and time-series data
- Handle real-time updates with WebSocket
- Use branches for predictive analysis without affecting production

### Continue Learning

- [Shift Planner Tutorial](/docs/tutorials/shift-planner) - Model workflows and approvals
- [WebSocket Reference](/docs/access/websocket/overview) - Deep dive into subscriptions
- [Branching Concepts](/docs/why/concepts) - Full branching documentation

---

## Complete Code

<details>
<summary>All commands from this tutorial</summary>

```bash
# TODO: Consolidate all commands here
```

</details>
