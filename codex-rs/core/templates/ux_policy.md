# UX policy

## Scope and responsibility

This policy is mandatory for all UI work, including design, implementation,
reviews, interaction changes, responsive layouts, accessibility, and user-facing
copy. The main agent owns compliance across the delivered journey; the UX agent
must apply the same policy to its assigned work. Explicit user directions take
precedence.

Bug fixes are exempt from this design workflow. Keep them focused on the defect,
preserve the established design, and verify the affected behavior. A redesign
bundled into a bug fix is UI work and must follow this policy.

## The standard: guide the next useful decision

**Recognize → choose → see the result → take the next useful step.**

People should immediately understand what a screen is for, recognize their
options, and know what happens next. They should not have to decode terminology,
search a form for its purpose, or assemble a journey from unrelated screens.

Clarity, comfortable spacing and predictable behavior must continue throughout
the product. A polished welcome screen followed by confusing working screens
fails this policy.

**Simplicity means less effort and uncertainty—not smaller controls, fewer words
at any cost, or hiding everything behind extra clicks.**

## 1. Start with the journey

Before designing a screen, establish:

- Who is here, and what are they trying to accomplish now?
- What do we already know, and what decision is actually needed?
- What useful result will this action produce?
- How do they continue, go back, leave and resume?

Design around that sequence, not around database fields, API endpoints or the
organization of the code. Reuse working capabilities. Do not invent new services,
requirements or business processes to make a layout feel complete.

## 2. Give each screen one clear job

- Lead with the task, content or decision. Use a short, familiar heading.
- Establish one primary action for the current task. Place it beside or directly
  after the content it acts on. Supporting actions should look secondary.
- Separate browsing existing items from creating a new one. Do not leave a blank
  creation form open beneath active work or mix a chooser into an existing item.
- Use a short guided sequence when decisions depend on earlier answers. Combine
  related details; do not create a mandatory step for every field.
- Choose the surface that fits the work. Setup may benefit from a wizard;
  conversation, writing, comparison and ongoing work usually need direct access
  to their content. Do not turn everything into a wizard.

## 3. Make choices obvious and easy to recognize

- Prefer readable whole-row or whole-card choices with a prominent title, a
  short explanation when needed, and a clear interaction cue.
- Make the whole choice usable. Do not require people to find a tiny radio button
  or text link inside a large decorative block.
- Use a small set of meaningful choices. Two to four is a useful starting point,
  not a quota. Keep an additional useful option visible when hiding it adds work.
- If choosing an option simply opens the next step, open it immediately. Avoid
  a redundant select-then-Continue click. Use Continue when people need to review
  multiple selections or adjust settings before proceeding.
- Distinguish navigation from selection. Selections need a clear state beyond
  color, such as a check and border. Unselected options must not look selected.
- Treat a custom option as a proper peer of the other choices. Do not wedge it
  beside an unrelated footer action. Keep the route back to choices discoverable.
- Within clickable content, keep independent actions independent: an edit,
  favorite or menu control must not also trigger the container's navigation.

## 4. Show useful results before exposing forms

For every input, ask in order: **can we import, derive, default, make optional,
or defer it?** Ask now only when the answer is necessary for the current step.

- Begin with useful examples, existing content or editable starting points.
  Prefer recognizing and adjusting to composing from a blank page.
- Show a readable preview with an obvious Edit action. Do not expose every
  editable field merely because it exists.
- Reveal dependent questions after the relevant choice. Omit irrelevant ones.
- Put uncommon settings and history behind clearly named disclosures. Keep
  necessary information and the primary action visible.
- Defaults must be useful and justified. Do not invent amounts, commitments or
  preferences that the person should decide. Support deferral when the workflow
  permits it, and reuse decisions already made elsewhere.
- Make optional steps clearly skippable, with a predictable result. Never replace
  a deliberate answer silently.

## 5. Create hierarchy with spacing and restraint

- Make the task and next action the focal point. Titles, choice labels, supporting
  explanations and metadata must have visibly different importance.
- Use consistent spacing to group related content and separate decisions.
  Comfortable spacing is functional; large empty bands are not.
- Give short decisions a comfortable reading width. Use more space when the
  content benefits from it. Do not stretch a small form across a wide screen.
- Use the product's established typography, components and visual language.
  Reuse its strongest approved patterns across the entire journey.
- Do not introduce Material Design or flat themes. Choose a visual language with
  deliberate hierarchy, depth, typography, and purposeful detail.
- Keep surfaces, borders, shadows, icons and color purposeful. Avoid nested
  cards, competing accents and a wall of equally prominent controls.
- Preserve approved copy during layout changes. Improve wording when the task
  calls for it, not as an incidental redesign side effect.

## 6. Protect useful screen space

**Budget the entire vertical stack, including the application header. The work
must dominate—not headings, introductions, tabs, filters and status strips.**

- Put the first useful choice or working content near the top. Short decision
  screens should show their options and next action in the initial viewport.
- Remove duplicated context, unnecessary toolbars and decorative framing before
  reducing useful space. Disclose secondary controls instead of crowding them in.
- Never fix overflow by shrinking text or touch targets, cramping controls, or
  placing buttons in unfamiliar arrangements.
- Long content may scroll naturally. Do not make people scroll past scaffolding
  before they can begin. Keep the next action easy to find without covering work.
- Reflow the hierarchy on mobile; do not squeeze the desktop layout. Account for
  the onscreen keyboard and browser chrome. Sticky controls must not obscure
  content, focused inputs or other actions.

## 7. Make navigation conventional and preserve work

- Put Back at the start of a nested task, conventionally the upper left, with a
  clear label. Root destinations do not need arbitrary Back buttons.
- Back returns to the previous useful context with its answers intact. Browser
  Back, Forward and refresh must also behave sensibly.
- Keep independent drafts for different items and templates. Switching options
  must not overwrite work. Scope drafts to the correct account and task.
- Preserve intent through authentication, setup, external steps and temporary
  detours. Return to the unfinished task rather than a generic dashboard.
- Carry established choices forward. Do not repeatedly ask for the same details.
- Make the distinction between draft, saved, sent and completed understandable.
  Failed requests must retain work and offer a clear retry or recovery path.
- After a consequential action, open the useful next state. Do not make people
  hunt through a list for the item they just created or acted on.

## 8. Use plain language and accurate feedback

- Do not be wordy. Use the fewest words that make the decision and result clear.
- State each point once. Do not repeat the same message from multiple angles in
  headings, subtitles, helper text, cards, and status messages.

- Label actions by their result: “Save changes,” “Send request,” “Publish.” Use
  “Continue” when the next step is already clear.
- Name capabilities and counts in customer language. Avoid internal bundle names,
  unexplained metrics and vague actions such as “Load deeper activity.”
- Explain an unfamiliar input beside it in one short sentence when necessary.
  Longer help can be disclosed. Do not surround simple tasks with instructional
  paragraphs or implementation details.
- Show the facts needed to understand a commitment at the point of action.
  Keep scope, destination, cost and consequence concise and contextual.
- Provide meaningful loading, empty, error and success states. Show actual
  progress; do not claim completion before it happens.
- Make recovery part of the flow. Avoid dead ends when useful intent can be
  retained for the next supported step.

## 9. Design accessibility into the interaction

Use readable contrast and type, meaningful labels, visible keyboard focus and
comfortable touch targets. Preserve logical reading and tab order. Support
keyboard activation of choices and disclose expanded states correctly. Do not
rely on color, hover or motion alone. Respect reduced-motion preferences.

## 10. Generate, compare, implement, verify

For new interfaces or substantial layout/interaction changes:

1. Trace the real journey and inspect existing capabilities.
2. Generate at least three meaningfully different visual compositions. Include
   the app shell, realistic populated content, actual labels and the next state.
   Written wireframes and screenshots of already-written code do not replace
   this exploration. If generation is unavailable, report it explicitly.
3. Compare task clarity, useful decisions, hierarchy, effort, continuity, control,
   accessibility and responsive fit. Score each 1–5; select a direction at 32/40
   or higher with nothing below 3. Record concrete weaknesses and why it wins.
   Behavior inferred from a mock remains provisional until tested.
4. Implement the chosen hierarchy and flow, not merely its colors. Include empty,
   loading, error, recovery and saved states.
5. Inspect the populated running interface at **1366×768, 1280×600 and 390×844**,
   plus relevant platform sizes and supported themes. Check long labels, realistic
   content, focus, overflow and the complete initial fold. A cropped or scrolled
   screenshot cannot prove that the first screen works.
6. Complete the affected journey through the real UI and application behavior.
   Verify backtracking, template changes, refresh, saved work, errors and return
   paths. For workflows involving multiple participants, verify both sides.
   A passing build or isolated component test does not prove the journey works.
7. Record the selected mock, rendered evidence, checks performed and remaining
   limits. Distinguish simulated or mocked checks from real external operations.

Routine behavior fixes do not require new mock generation. Verify their affected
journeys directly. A substantial redesign does not become exempt by calling it
a bug fix.

## Final review: reject these regressions

| Reject | Require instead |
| --- | --- |
| Every possible field visible at once | Focused decisions, useful previews, optional editing |
| Tiny selectable label in a decorative block | Readable, usable whole-choice target |
| Existing work mixed with unfinished creation | Distinct list, setup and working states |
| Several equally prominent actions | A clear next action with supporting alternatives |
| Back loses work or changes context | Predictable return with the same draft |
| Repeated setup during a task | Reuse known choices and return to current work |
| Empty space fixed by adding unrelated controls | Space organized around the actual task |
| Everything fits only because it is smaller | Less scaffolding and fewer simultaneous decisions |
| Attractive screenshots but an unfinished journey | Verified completion, recovery and continuity |

**The person should be able to finish the job without learning how the product
is implemented. That is the measure of simplicity.**

## Expanded states must be simple too

A disclosure is not a substitute for simplifying the task. Review every expanded state: remove redundant questions, derive known values, and separate unrelated tasks. Avoid nested accordions and large forms hidden under vague labels such as “More details.” Use a focused editor with a clear Back and Save when optional work needs several decisions.
