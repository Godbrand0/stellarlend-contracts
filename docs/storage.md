# StellarLend Storage Layout and Migration Guide

This document describes the persistent storage structure of the StellarLend protocol on Soroban.

## Overview

StellarLend uses Soroban's `persistent()` storage for all long-term data.
All keys are defined using `contracttype` enums.

> [!IMPORTANT]
> **Namespace Isolation**: To prevent collisions between modules, all storage key enum variants MUST be unique across the entire contract. Even different enum types will collide if their variants share the same name (as they serialize to the same `Symbol`).

## Storage Map

### 1. Cross-Asset Core (`cross_asset.rs`)

| Key Type | Variant | Value Type | Description |
|----------|---------|------------|-------------|
| `CrossAssetDataKey` | `CrossAssetAdmin` | `Address` | Protocol admin for cross-asset operations. |
| `CrossAssetDataKey` | `CrossAssetPaused` | `bool` | Pause flag for cross-asset module. |
| `CrossAssetDataKey` | `AssetParams(Address)` | `AssetParams` | Config for a specific asset. |
| `CrossAssetDataKey` | `UserPosition(Address)` | `UserPosition` | User collateral and debt. |

### 2. Oracle Module (`oracle.rs`)

| Key Type | Variant | Value Type | Description |
|----------|---------|------------|-------------|
| `OracleKey` | `OraclePaused` | `bool` | Pause flag for oracle updates. |
| `OracleKey` | `Config` | `OracleConfig` | Global oracle settings. |
| `OracleKey` | `PrimaryOracle(Address)` | `Address` | Primary feed for an asset. |

### 3. Data Store (`data_store.rs`)

| Key Type | Variant | Value Type | Description |
|----------|---------|------------|-------------|
| `StoreKey` | `StoreAdmin` | `Address` | Admin for the DataStore module. |
| `StoreKey` | `Entry(String)` | `Bytes` | Dynamic key-value entries. |

### 4. Withdraw Module (`withdraw.rs`)

| Key Type | Variant | Value Type | Description |
|----------|---------|------------|-------------|
| `WithdrawDataKey` | `WithdrawPaused` | `bool` | Legacy pause flag for withdrawals. |

## Collision Prevention Rules

1.  **Unique Variant Names**: All enum variants used as storage keys must be prefixed with their module name (e.g., `OraclePaused` instead of `Paused`).
2.  **Audit Tests**: Every new storage key must be added to `storage_collision_test.rs` to verify isolation.
3.  **Tuple Namespacing**: For highly dynamic or multi-tenant keys, use a tuple `(Symbol, EnumVariant)` to guarantee isolation.

## Audit History

| Date | Auditor | Findings | Action |
|------|---------|----------|--------|
| 2026-04-26 | Antigravity | Collisions found in `Paused` and `Admin` variants. | Renamed variants to include module prefixes. |
