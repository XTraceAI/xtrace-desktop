# Design token contract

`token-contract.json` defines the shared colors, typography, corner radii and
shadows. Tests compare the dark/light styles with these values so new components
use a consistent appearance.

Changes to the palette must update the contract and implementation together
and explain the design change. Runtime styles live in
`apps/desktop/ui/src/styles/tokens.css`.
