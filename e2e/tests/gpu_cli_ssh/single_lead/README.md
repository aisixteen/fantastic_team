# gpu_cli_ssh/single_lead

`cli_ssh/single_lead` on a real model (the gpu tier, #237, #242, #252): the vllm plug, no script. Parts A (the lead made with `create_agent`) and B (fantastic's recipe, the root AI `ai`), each with a box of its own. How the lead is told, the SKIP rules and the box are in `e2e/README.md` ("The twins").

```
python3 e2e/run.py --only gpu_cli_ssh/single_lead --bin /ABSOLUTE/path/fantastic_team
```
