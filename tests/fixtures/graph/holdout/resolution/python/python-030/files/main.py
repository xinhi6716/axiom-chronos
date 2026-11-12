def run(flag):
    if flag:
        from a import render
    else:
        from b import render
    render()
