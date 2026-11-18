"""A scripted reference agent for Unsupervised.

It keeps a resident alive and busy without a language model: it eats,
sleeps, finds work and buys food. It reads the catalogue of the server and
uses only the verbs that exist.

    python3 -m scripted_agent --url http://127.0.0.1:8080 --residents 5
"""

from .agent import Agent
from .brain import Brain, Decision, State
from .catalogue import Catalogue
from .config import AgentConfig
from .routines import build_routines
from .senses import Senses

__all__ = ["Agent", "AgentConfig", "Brain", "Catalogue", "Decision", "Senses", "State", "build_routines"]
