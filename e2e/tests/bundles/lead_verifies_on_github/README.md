# bundles/lead_verifies_on_github (free, scripted)

team#289: a lead verifies a result on GitHub (`pr_status` through the github connector), never from the local clone. The stock lead is told its task is finished; the scripted lead asks the fake forge for pull request 7 and reports the task done from the answer.

Asserts: the forge saw the `pr_status` reads (the pull request, its check runs and its reviews); the lead's next request holds the forge's answer (`open`, the head sha); the lead's last answer reports the task done, not failed; no local `git` call was made by it.
