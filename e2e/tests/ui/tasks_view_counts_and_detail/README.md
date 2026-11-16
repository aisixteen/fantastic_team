# ui/tasks_view_counts_and_detail (free)

The task registry's view, opened from the tree (#351). Function only.

13 tasks are made with the registry's verbs before the browser opens (every state: 2 waiting, 3 running, 1 blocked, 5 done, 1 failed, 1 paused; holders: agents, two subtrees, nobody). Checked on the page: the counts per state (failed and paused included), the groups and rows with holder and state, "2 more done", the status and holder filters (alone, together, cleared), a task's detail (source, holder, version, the two history steps, the last report's sender, state and text), another task's detail, and LIVE updates: the view watches the registry and sets no timer (its served source is read), and a state change and a new task made in the registry outside the page (operator calls) show within 3 s without a reload (the poll it replaced re-read every 5 s).
