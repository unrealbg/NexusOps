# ADR 0003: Bounded read-only operations

Status: Accepted

Goal 01 allows only fixed read-only probe commands. An operation policy validates read-only risk before transport execution. Future mutating operations must extend explicit plan, validate, execute, verify and optional rollback boundaries. No remote installation is needed.

Discovery composes small HostProbe implementations with independently testable parsers. Each command has a timeout, cancellation and output budget. Probe failures are represented as safe warnings, not terminal connection failures. Audit events record identifiers, actor, outcome and duration, never commands, credentials or command output.
