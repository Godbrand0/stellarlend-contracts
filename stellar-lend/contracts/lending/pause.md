# Protocol Pause Mechanism

The StellarLend protocol includes a granular pause mechanism to ensure safety during emergency situations or maintenance.

## Features

- **Granular Control**: Pause specific operations (`Deposit`, `Borrow`, `Repay`, `Withdraw`, `Liquidation`) without affecting others.
- **Global Pause**: A master switch (`All`) to pause the entire protocol immediately.
- **Admin Managed**: Only the protocol admin can toggle pause states.
- **Guardian Trigger**: A configured guardian can trigger emergency shutdown without waiting for full governance latency.
- **Recovery Mode**: After shutdown, admin can move protocol into controlled unwind mode for user exits.
- **Event Driven**: All pause state changes emit `pause_changed` events for transparency.

## Operation Types

The protocol supports the following `PauseType` values:

| Enum Value    | Description                                                       |
| ------------- | ----------------------------------------------------------------- |
| `All`         | Global pause affecting all operations listed below.               |
| `Deposit`     | Prevents users from depositing new collateral.                    |
| `Borrow`      | Prevents users from taking out new loans.                         |
| `Repay`       | Prevents users from repaying loans (should be used with caution). |
| `Withdraw`    | Prevents users from withdrawing collateral.                       |
| `Liquidation` | Prevents liquidations from being performed.                       |

## Contract Interface

### Admin Functions

#### `set_pause(admin: Address, pause_type: PauseType, paused: bool)`

Toggles the pause state for a specific operation or the entire protocol.

- **Requires Authorization**: Yes (by `admin`).
- **Emits**: `pause_changed` event.

#### `set_guardian(admin: Address, guardian: Address)`

Sets or rotates the guardian authorized to trigger emergency shutdown.

- **Requires Authorization**: Yes (by `admin`).
- **Emits**: `guardian_set_event`.

#### `start_recovery(admin: Address)`

Transitions the protocol from `Shutdown` to `Recovery`.

- **Requires Authorization**: Yes (by `admin`).
- **Precondition**: Emergency state must be `Shutdown`.
- **Emits**: `emergency_state_event`.

#### `complete_recovery(admin: Address)`

Transitions the protocol from `Recovery` (or `Shutdown`) back to `Normal`.

- **Requires Authorization**: Yes (by `admin`).
- **Emits**: `emergency_state_event`.

### Guardian/Admin Emergency Function

#### `emergency_shutdown(caller: Address)`

Transitions protocol to `Shutdown`.

- **Requires Authorization**: Yes (by `admin` or configured `guardian`).
- **Emits**: `emergency_state_event`.

### Public Functions

#### `get_admin() -> Option<Address>`

Returns the current protocol admin address.

#### `get_guardian() -> Option<Address>`

Returns the currently configured guardian, if any.

#### `get_emergency_state() -> EmergencyState`

Returns current emergency lifecycle state:

- `Normal`: standard operation
- `Shutdown`: hard stop of high-risk operations
- `Recovery`: controlled unwind where users can repay and withdraw, but cannot open new risk

## Security Assumptions

1. **Admin Trust**: The admin is assumed to be a multisig or a DAO-governed address to prevent centralization risks.
2. **Persistence**: Pause states are stored in persistent storage to survive ledger upgrades and contract updates.
3. **No Bypass**: User entrypoints and token-receiver execution paths enforce pause/emergency checks.
4. **Least-Risk Recovery**: During `Recovery`, only unwind operations (`repay`, `withdraw`) remain open.
5. **Defense in Depth**: Emergency checks are enforced at both contract entry and core borrow logic.

## Usage Example (Rust SDK)

```rust
// Pause borrowing in an emergency
client.set_pause(&admin, &PauseType::Borrow, &true);

// Re-enable borrowing
client.set_pause(&admin, &PauseType::Borrow, &false);
```
