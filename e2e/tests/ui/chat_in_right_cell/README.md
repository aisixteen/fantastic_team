# ui/chat_in_right_cell (free)

The team page's right cell draws the BASE chat view (fantastic#609, team #260): a click on an agent in the left cell (the base sidebar view) opens it, and the page asks that agent for its `reflect`, then mounts the view it ships through the same base relay module (`view_relay.js`, `mountView`) the left cell uses. The page holds no chat or relay code of its own; the leg `team_ws` holds exactly the chat view's calls on the agent (`watch reflect history rows status send stop context_clear unembed`), never `*`.

Playwright (Chrome) opens `/team_page/file/index.html` against the team's real binary (`webui --port N`, scripted model, no key) and asserts, reading the views' documents (null-origin iframes):

- the lead unfolds in the left cell and a click on its child `w1` (an `ai_chat` on the scripted model) opens it: `#body-right[data-open=w1]`;
- the right cell holds an iframe whose document is the chat view (`#text`, `#send`);
- a message typed in `#text` and sent with `#send` shows as a person bubble (`.who.user`) and the scripted reply as an agent bubble (`.who.assistant`);
- the message reached the agent: the scripted model got one request holding the token, and the agent's history (read over the page's leg) holds it;
- no console error.
