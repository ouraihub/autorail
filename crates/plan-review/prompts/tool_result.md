<!-- What the worker reads after a review (host/requestReview.ts composeText): the request_review tool
     result on pi/OMP, the message after the 【评审结论】 prefix on DSH. -->
## approved
Review verdict: APPROVE — step "{title}".
## approve_blocked
Review verdict: APPROVE — step "{title}" — but it is NOT settled: {open} earlier finding(s) on this step still open. Resolve each one you actually addressed with resolve_comment, then request_review again.
## changes
Review verdict: REQUEST_CHANGES — step "{title}".
## fix_now
Fix each finding below NOW without asking the user: set the step back to in_progress (todo_update), apply the fixes, mark it done, then call request_review again.
## stop
The automated fix cycle is spent or auto-fix is off: do NOT fix now. Report these findings to the user and wait for their direction.
## failed
Review of step "{title}" failed: {message}
