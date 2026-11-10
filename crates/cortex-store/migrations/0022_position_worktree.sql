-- Where each position was taken: the linked working trees of a repository share its store, so a
-- position names the working tree it was taken in (`main`, or `linked:` followed by git's name for
-- it, nothing after the colon when the name could not be read) and the branch that working tree's
-- HEAD named then (NULL when it was detached or could not be read). NULL working tree: not known,
-- as for every position kept before. Names only, never a path.

ALTER TABLE positions ADD COLUMN worktree TEXT;
ALTER TABLE positions ADD COLUMN branch TEXT;
