<!-- Lines of the reviewer's first message (todos/todos.ts renderReviewPackage + REVIEWER_OUTPUT_CONTRACT).
     Each `## key` section is one line; `{name}` is replaced by the host. -->
## step
Plan step {id} ("{title}") of chat {chat} is done and awaits review.
## commit
Commit {sha} ("{title}") of chat {chat} belongs to no plan step and awaits review.
## note
Step note: {note}
## summary
Worker's completion summary: {summary}
## verification
Worker's verification claim: {verification} (verify the claim, don't trust it)
## no_verification
Worker reported NO verification — weigh that in your review.
## change_set
Change set: {change_set}
## rereview
RE-REVIEW: only {fresh} is new since the last verdict — review only that delta. Earlier findings the fix addressed are resolved by the worker or excluded as stale; approve is blocked only by what's still open.
## contract
Review the change set above. Reply with your verdict as the single fenced json block described in your instructions.
