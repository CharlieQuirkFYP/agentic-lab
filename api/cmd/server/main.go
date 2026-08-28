package main

import (
	"log"

	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/analyzer"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/handler"
	"github.com/CharlieQuirkFYP/agentic-lab/api/internal/service"
	"github.com/gin-gonic/gin"
)

func main() {
	router := gin.Default()

	incidentAnalyzer := analyzer.NewMockIncidentAnalyzer()
	incidentService := service.NewIncidentService(incidentAnalyzer)
	incidentHandler := handler.NewIncidentHandler(incidentService)

	router.GET("/health", handler.Health)
	router.POST("/api/v1/incidents/analyze", incidentHandler.Analyze)

	log.Fatal(router.Run(":8080"))
}
