"""Reading, checking and publishing task repos.

A task repo is the source of truth for a dataset: one directory per instance,
holding everything needed to run and grade it. `repo` reads the tree, `checks`
says whether it is well formed, and `publish` turns it into a dataset and images.
"""
