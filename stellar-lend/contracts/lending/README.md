# StellarLend Lending Contract

A secure, efficient lending protocol built on Soroban that allows users to borrow assets against collateral.

## Features

- **Collateralized Borrowing**: Borrow assets by providing collateral with a minimum 150% ratio
- **Interest Accrual**: Automatic interest calculation at 5% APY
- **Debt Ceiling**: Protocol-level debt limits for risk management
- **Collateralized Borrowing**: Borrow assets by providing collateral with a minimum 150% ratio
- **Interest Accrual**: Automatic interest calculation at 5% APY
- **Debt Ceiling**: Protocol-level debt limits for risk management
- **Pause Mechanism**: Granular emergency pause functionality for specific operations (Deposit, Borrow, Repay, Withdraw, Liquidation)
- **Emergency Lifecycle**: `Normal -> Shutdown -> Recovery -> Normal` flow with guardian-triggered shutdown and admin-controlled recovery
- **Admin Control**: Secure protocol management with a dedicated admin role
- **Overflow Protection**: Comprehensive checks against arithmetic overflow
- **Event Emission**: Track all borrow and pause operations via events

## Building

```bash
cargo build --target wasm32-unknown-unknown --release
```

## Testing

```bash
cargo test
```

## Documentation

See [borrow.md](./borrow.md), [pause.md](./pause.md), and [emergency_shutdown.md](./emergency_shutdown.md) for comprehensive documentation including:

- Function signatures and parameters
- Error types and handling
- Security assumptions
- Usage examples
- Best practices

## Contract Interface

### Main Functions

- `borrow()` - Borrow assets against collateral
- `get_user_debt()` - Query user's debt position
- `get_user_collateral()` - Query user's collateral position

### Admin Functions

- `initialize()` - Set protocol admin, debt ceiling, and minimum borrow amount
- `set_pause()` - Granular pause for specific operations
- `set_guardian()` - Configure emergency guardian
- `emergency_shutdown()` - Trigger hard emergency shutdown (admin or guardian)
- `start_recovery()` - Enter controlled user unwind mode (admin only)
- `complete_recovery()` - Return protocol to normal operation (admin only)
- `get_admin()` - Returns the current protocol admin

## Security

- Minimum 150% collateral ratio enforced
- All arithmetic operations use checked methods
- Authorization required for all user operations
- Comprehensive test coverage including edge cases

## License

See repository root for license information.
