# ADR 0087: DXGI-only product capture routing

Date: 2026-11-17. Supersedes ADR 0084's WGC-first product policy and ADR 0086's
WGC fallback option, at the user's explicit request to avoid WGC outlines entirely.

The production DisplayCapture router uses Desktop Duplication for displays and
guarded DXGI cropping for game/window selections, including legacy identifiers.
No failure starts WGC. UI labels disclose cropped-monitor semantics and the
focus-away frozen-frame behavior. Excluded targets remain denied.

WGC implementation and direct hardware fixtures are retained as historical code,
not product-selectable backends. Historical ignored tests that assert
DisplayCapture uses `wgc-window` need conversion to direct WGC fixtures before
reuse; they are not acceptance tests for this new routing. Native freeze/resume
acceptance and installer rebuild remain pending. This change does not establish
all-frame privacy or equivalence to render-pipeline game capture.
