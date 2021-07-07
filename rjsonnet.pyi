from typing import Dict, Optional, Union, List, Tuple, Callable


def evaluate_file(
    filename: str,
    jpathdir: Optional[Union[str, List[str]]] = None,
    max_stack: int = 500,
    gc_min_objects: int = 1000,
    gc_growth_trigger: float = 2.0,
    ext_vars: Dict[str, str] = {},
    ext_codes: Dict[str, str] = {},
    tla_vars: Dict[str, str] = {},
    tla_codes: Dict[str, str] = {},
    max_trace: int = 20,
    import_callback: Optional[Callable[[str, str], Tuple[str, Optional[str]]]] = None,
    native_callbacks: Dict[str, Tuple[str, Callable]] = {},
) -> str: ...


def evaluate_snippet(
    filename: str,
    snippet: str,
    jpathdir: Optional[Union[str, List[str]]] = None,
    max_stack: int = 500,
    gc_min_objects: int = 1000,
    gc_growth_trigger: float = 2.0,
    ext_vars: Dict[str, str] = {},
    ext_codes: Dict[str, str] = {},
    tla_vars: Dict[str, str] = {},
    tla_codes: Dict[str, str] = {},
    max_trace: int = 20,
    import_callback: Optional[Callable[[str, str], Tuple[str, Optional[str]]]] = None,
    native_callbacks: Dict[str, Tuple[str, Callable]] = {},
) -> str: ...
