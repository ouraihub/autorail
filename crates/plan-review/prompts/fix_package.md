<!-- The message the user's 发给 Agent 修复 sends to the worker (todos/todos.ts renderFixPackage); the
     feedback is the review's findings. -->
## head
The user reviewed your completed step {id} ("{title}") and asked for a fix.
## note
Step note: {note}
## summary
Your completion summary: {summary}
## verification
Your verification claim: {verification}
## change_set
Change set under review: {change_set}
## feedback
User feedback:
## step
Address the feedback on THIS step: flip {id} back to in_progress (todo_update), make the fix, then mark it done with a fresh summary AND a fresh commitSubject describing the fix (the revision is its own commit). Do not create a new item for it.
## commit
Address the feedback by revising the change in {change_set}: make the fix and commit it. This commit belongs to no plan step, so there is nothing to re-open — your follow-up commit surfaces as a new entry. Leave unrelated commits untouched.
