"""Estadísticas descriptivas mínimas para el ejemplo."""
# Solo usa la biblioteca estándar de Python.
# Las funciones trabajan sobre listas de números.

def mean(valores):
    """Devuelve la media aritmética de los valores."""
    return sum(valores) / len(valores)

def variance(valores):
    """Devuelve la varianza poblacional de los valores."""
    media = mean(valores)
    return sum((x - media) ** 2 for x in valores) / len(valores)

def summary(valores):
    """Devuelve un dict con cuenta, media y varianza."""
    return {"n": len(valores), "media": mean(valores), "varianza": variance(valores)}

if __name__ == "__main__":
    datos = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]
    print(summary(datos))
