---
version: "alpha"
name: "Origami Geométrico"
description: "Design an origami geometric landing page. Ideal for landing pages, saas. AI-ready template."
colors:
  primary: "#FAFAFA"
  secondary: "#B0B0B0"
  tertiary: "#1A1A1A"
  neutral: "#FF6B6B"
  surface: "#87CEEB"
  accent: "#A8D5BA"
typography:
  h1:
    fontFamily: Poppins
    fontSize: 2.5rem
    fontWeight: 700
  body-md:
    fontFamily: Poppins
    fontSize: 1rem
    fontWeight: 400
components:
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.neutral}"
    padding: 12px
---

## Overview

Design an origami geometric landing page. Ideal for landing pages, saas. AI-ready template. Paper folding has been whispering to designers for centuries. Long before screens existed, origami demonstrated something profound: a single flat sheet, through nothing but precise folds, becomes dimensional. That's not craft — that's mathematics made tangible.

The Bauhaus crowd noticed. So did the Constructivists. But it was the Japanese tradition that kept the discipline pure — no cuts, no glue, just geometry and intention. When digital design matured enough to handle angular complexity, origami's visual language migrated naturally into interfaces, logos, and spatial branding. Those crisp valley folds and mountain folds? They translate directly into light, shadow, and the illusion of depth on flat screens.

What makes origami geometry endlessly compelling is its constraint. Every fold follows mathematical rules — the Huzita-Hatori axioms aren't suggestions, they're laws. And constraint, as any designer worth their salt knows, is where creativity actually lives. The angular facets, the interplay of planes catching light differently — it's structured beauty that never feels arbitrary.

- Density: 5/10 — Balanced
- Variance: 4/10 — Moderate
- Motion: 4/10 — Subtle

- **Style:** Folded, Angular, Precise
- **Keywords:** origami, geometric, folded, angular, precise, paper craft, tessellation, faceted, low-poly, mathematical
- **Era:** Contemporary Paper Art Meets Digital
- **Light/Dark:** ✓ Full / ✗ No

## Colors

- **Paper White** (#FAFAFA) — Light surface, card backgrounds
- **Fold Shadow** (#B0B0B0) — Secondary surface or text color
- **Ink Black** (#1A1A1A) — Dark surface, primary background
- **Accent Coral** (#FF6B6B) — Primary accent, CTAs and interactive elements
- **Sky Fold** (#87CEEB) — Extended palette, decorative use
- **Sage Paper** (#A8D5BA) — Extended palette, decorative use
- **Warm Crease** (#F0C987) — Extended palette, decorative use
- **Steel Grey** (#4A4A4A) — Secondary text, borders, muted elements


## Typography

- **Display / Hero:** Poppins — Weight 700, tight tracking, used for headline impact
- **Body:** Poppins — Weight 400, 16px/1.6 line-height, max 72ch per line
- **UI Labels / Captions:** Poppins — 0.875rem, weight 500, slight letter-spacing
- **Monospace:** JetBrains Mono — Used for code, metadata, and technical values

Scale:
- Hero: clamp(2.5rem, 5vw, 4rem)
- H1: 2.25rem
- H2: 1.5rem
- Body: 1rem / 1.6
- Small: 0.875rem


## Layout

- **Grid:** CSS Grid primary. Max-width containment: 1280px centered with 1.5rem side padding.
- **Spacing rhythm:** Balanced. Base unit: 0.5rem (8px).
- **Section vertical gaps:** clamp(4rem, 8vw, 8rem).
- **Hero layout:** Split-screen (text left, visual right).
- **Feature sections:** Zig-zag alternating text+image rows. No 3-equal-columns.
- **Mobile collapse:** All multi-column layouts collapse below 768px. No horizontal overflow.
- **z-index contract:** base (0) / sticky-nav (100) / overlay (200) / modal (300) / toast (500).


## Elevation & Depth

CSS polygon clip-paths, faceted card surfaces, paper fold shadows, tessellation backgrounds, angular section dividers, crease line borders, layered paper depth, geometric hover transforms

- **Physics:** Ease-out curves, 200-300ms duration. Smooth and predictable.
- **Entry animations:** Fade + translate-Y (16px → 0) over 420ms ease-out. Staggered cascades for lists: 80ms between items.
- **Hover states:** Subtle color shift + shadow adjustment over 200ms.
- **Page transitions:** Fade only (200ms).
- **Performance:** Only transform and opacity animated. No layout-triggering properties.


## Shapes

Base corner radius: 0px. See rounded tokens in front matter for the full scale.


## Components

- **Primary Button:** Sharp edges (0px) shape. Accent color fill. Hover: 8% darken + subtle lift shadow. Active: -1px translate tactile press. Font weight 600. No outer glows.
- **Secondary / Ghost Button:** Outline variant. 1.5px border in muted color. Text in primary color. Hover: subtle background fill.
- **Cards:** Sharp edges (0px) corners. Surface background. Subtle shadow (0 2px 12px rgba(0,0,0,0.06)). 1px border stroke.
- **Inputs:** Label above input. 1px border stroke. Focus ring: 2px accent color offset 2px. Error text below in semantic red. No floating labels.
- **Navigation:** Primary surface background. Active item: accent color indicator. Font weight 500 when active.
- **Skeletons:** Shimmer animation matching component dimensions. No circular spinners.
- **Empty States:** Icon-based composition with descriptive text and action button.


## Do's and Don'ts

- No emojis in UI — use icon system only (Lucide, Heroicons)
- No pure black (#000000) — use off-black or charcoal variants
- No oversaturated accent colors (saturation cap: 80%)
- No 3-column equal-width feature layouts — use zig-zag or asymmetric grid
- No `h-screen` — use `min-h-[100dvh]`
- No AI copywriting clichés: "Elevate", "Seamless", "Unleash", "Next-Gen"
- No broken external image links — use picsum.photos or inline SVG
- No generic lorem ipsum in demos

- Do CSS polygon clip-paths
- Do Faceted card surfaces
- Do Paper fold shadows
- Do Tessellation backgrounds
- Do Angular section dividers
- Do Geometric hover transforms


## Use Case

Landing pages, SaaS

<!-- Source: https://designmd.app/library/origami-geometrico · designmd.app -->
